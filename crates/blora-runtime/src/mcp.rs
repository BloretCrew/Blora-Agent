// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! MCP client manager: multiple servers, tools, resources, and prompts.
//!
//! Configuration is read from `BLORA_MCP_CONFIG`, `<workspace>/.blora/mcp.json`,
//! and `$BLORA_HOME/mcp.json`, in that order. `BLORA_MCP_COMMAND` remains a
//! single-server compatibility fallback.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;

use blora_model::ToolDeclaration;
use blora_types::{BloraError, Result};
use serde::Deserialize;
use serde_json::{Value, json};

const PROTOCOL_VERSION: &str = "2024-11-05";
const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct McpServerConfig {
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub trusted: bool,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct McpConfigFile {
    #[serde(default)]
    servers: BTreeMap<String, McpServerConfig>,
}

struct McpServer {
    name: String,
    config: McpServerConfig,
    client: Mutex<Option<McpClient>>,
}

#[derive(Default)]
pub struct McpManager {
    servers: Vec<McpServer>,
}

#[derive(Clone, Debug)]
struct McpToolInfo {
    name: String,
    description: String,
    parameters: Value,
}

impl McpManager {
    /// Load configured servers. Returns `None` when nothing is configured or
    /// every server is disabled.
    #[must_use]
    pub fn load(workspace: &Path) -> Option<Self> {
        let mut configs = BTreeMap::new();
        for path in config_paths(workspace) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(file) = serde_json::from_str::<McpConfigFile>(&text) else {
                tracing::warn!("invalid MCP config: {}", path.display());
                continue;
            };
            configs.extend(file.servers);
        }
        if configs.is_empty() {
            if let Ok(command) = std::env::var("BLORA_MCP_COMMAND") {
                let mut parts = command.split_whitespace();
                if let Some(program) = parts.next() {
                    configs.insert(
                        "default".to_owned(),
                        McpServerConfig {
                            command: Some(program.to_owned()),
                            args: parts.map(str::to_owned).collect(),
                            ..McpServerConfig::default()
                        },
                    );
                }
            }
        }
        let servers = configs
            .into_iter()
            .filter(|(_, config)| config.enabled.unwrap_or(true))
            .map(|(name, config)| McpServer {
                name,
                config,
                client: Mutex::new(None),
            })
            .collect::<Vec<_>>();
        (!servers.is_empty()).then_some(Self { servers })
    }

    #[must_use]
    pub fn server_names(&self) -> Vec<String> {
        self.servers
            .iter()
            .map(|server| server.name.clone())
            .collect()
    }

    /// Tool declarations for all servers, using `mcp__server__tool` names.
    ///
    /// A single compatibility server named `default` keeps the old
    /// `mcp__tool` spelling.
    pub fn tool_declarations(&self) -> Vec<ToolDeclaration> {
        let single_default = self.servers.len() == 1 && self.servers[0].name == "default";
        let mut out = Vec::new();
        for server in &self.servers {
            let tools = match server.with_client(|client| client.list_tools()) {
                Ok(tools) => tools,
                Err(err) => {
                    tracing::warn!("MCP server {} tools/list failed: {err}", server.name);
                    continue;
                }
            };
            for tool in tools {
                let name = if single_default {
                    format!("mcp__{}", tool.name)
                } else {
                    format!("mcp__{}__{}", server.name, tool.name)
                };
                out.push(ToolDeclaration {
                    name,
                    description: tool.description,
                    parameters: tool.parameters,
                });
            }
        }
        out
    }

    pub fn call(&self, name: &str, arguments: &Value) -> Result<String> {
        let (server_name, tool) = self.split_tool_name(name)?;
        let server = self.server(&server_name)?;
        server.with_client(|client| client.call_tool(&tool, arguments))
    }

    pub fn list_resources(&self, server: Option<&str>) -> Result<String> {
        let mut out = Vec::new();
        for entry in self.selected(server)? {
            let resources = entry.with_client(|client| client.list_resources())?;
            out.push(format!("[{}]\n{}", entry.name, resources));
        }
        Ok(out.join("\n\n"))
    }

    pub fn read_resource(&self, server: &str, uri: &str) -> Result<String> {
        self.server(server)?
            .with_client(|client| client.read_resource(uri))
    }

    pub fn list_prompts(&self, server: Option<&str>) -> Result<String> {
        let mut out = Vec::new();
        for entry in self.selected(server)? {
            let prompts = entry.with_client(|client| client.list_prompts())?;
            out.push(format!("[{}]\n{}", entry.name, prompts));
        }
        Ok(out.join("\n\n"))
    }

    pub fn get_prompt(&self, server: &str, name: &str, arguments: &Value) -> Result<String> {
        self.server(server)?
            .with_client(|client| client.get_prompt(name, arguments))
    }

    fn server(&self, name: &str) -> Result<&McpServer> {
        self.servers
            .iter()
            .find(|server| server.name == name)
            .ok_or_else(|| BloraError::Other(format!("unknown MCP server: {name}")))
    }

    fn selected(&self, server: Option<&str>) -> Result<Vec<&McpServer>> {
        match server {
            Some(name) => Ok(vec![self.server(name)?]),
            None => Ok(self.servers.iter().collect()),
        }
    }

    fn split_tool_name(&self, name: &str) -> Result<(String, String)> {
        let rest = name
            .strip_prefix("mcp__")
            .ok_or_else(|| BloraError::Other(format!("not an MCP tool: {name}")))?;
        if let Some((server, tool)) = rest.split_once("__") {
            if self.servers.iter().any(|entry| entry.name == server) {
                return Ok((server.to_owned(), tool.to_owned()));
            }
        }
        if self.servers.len() == 1 {
            return Ok((self.servers[0].name.clone(), rest.to_owned()));
        }
        Err(BloraError::Other(format!(
            "MCP tool needs a server prefix: {name}"
        )))
    }
}

impl McpServer {
    fn with_client<T>(&self, action: impl FnOnce(&mut McpClient) -> Result<T>) -> Result<T> {
        let mut guard = self
            .client
            .lock()
            .map_err(|_| BloraError::Other("MCP client lock poisoned".to_owned()))?;
        if guard.is_none() {
            *guard = Some(McpClient::connect(&self.config)?);
        }
        action(guard.as_mut().expect("client initialized"))
    }
}

fn config_paths(workspace: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(path) = std::env::var("BLORA_MCP_CONFIG") {
        paths.push(PathBuf::from(path));
    }
    paths.push(workspace.join(".blora/mcp.json"));
    let home = std::env::var("BLORA_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".blora")));
    if let Some(home) = home {
        paths.push(home.join("mcp.json"));
    }
    paths
}

enum Transport {
    Stdio {
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<std::process::ChildStdout>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
        session_id: Option<String>,
    },
}

struct McpClient {
    transport: Transport,
    next_id: u64,
    timeout_secs: u64,
}

impl McpClient {
    fn connect(config: &McpServerConfig) -> Result<Self> {
        let timeout_secs = config.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let transport = if config.transport.as_deref() == Some("http") {
            let url = config
                .url
                .clone()
                .ok_or_else(|| BloraError::Other("MCP http server needs url".to_owned()))?;
            Transport::Http {
                url: expand_env(&url),
                headers: config
                    .headers
                    .iter()
                    .map(|(key, value)| (key.clone(), expand_env(value)))
                    .collect(),
                session_id: None,
            }
        } else {
            let command = config
                .command
                .clone()
                .ok_or_else(|| BloraError::Other("MCP stdio server needs command".to_owned()))?;
            let mut child = Command::new(&command)
                .args(&config.args)
                .envs(&config.env)
                .current_dir(config.cwd.as_deref().unwrap_or("."))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(BloraError::exec)?;
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| BloraError::Other("MCP stdin missing".to_owned()))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| BloraError::Other("MCP stdout missing".to_owned()))?;
            Transport::Stdio {
                child,
                stdin,
                stdout: BufReader::new(stdout),
            }
        };
        let mut client = Self {
            transport,
            next_id: 1,
            timeout_secs,
        };
        let _ = client.rpc(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "blora-agent", "version": "0.1.0"}
            }),
        )?;
        let _ = client.rpc("notifications/initialized", json!({}));
        Ok(client)
    }

    fn list_tools(&mut self) -> Result<Vec<McpToolInfo>> {
        let result = self.rpc("tools/list", json!({}))?;
        Ok(result
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .map(|tool| McpToolInfo {
                        name: tool
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_owned(),
                        description: tool
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("MCP tool")
                            .to_owned(),
                        parameters: tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| json!({})),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<String> {
        let result = self.rpc("tools/call", json!({"name": name, "arguments": arguments}))?;
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = extract_content(&result).unwrap_or_else(|| result.to_string());
        if is_error {
            return Err(BloraError::Other(format!("MCP tool {name} failed: {text}")));
        }
        Ok(text)
    }

    fn list_resources(&mut self) -> Result<String> {
        let result = self.rpc("resources/list", json!({}))?;
        let resources = result
            .get("resources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if resources.is_empty() {
            return Ok("(no resources)".to_owned());
        }
        Ok(resources
            .iter()
            .map(|resource| {
                format!(
                    "{}  {}  {}",
                    resource.get("uri").and_then(Value::as_str).unwrap_or(""),
                    resource
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    resource.get("name").and_then(Value::as_str).unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn read_resource(&mut self, uri: &str) -> Result<String> {
        let result = self.rpc("resources/read", json!({"uri": uri}))?;
        let contents = result
            .get("contents")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = String::new();
        for content in contents {
            if let Some(text) = content.get("text").and_then(Value::as_str) {
                out.push_str(text);
                out.push('\n');
            } else if let Some(blob) = content.get("blob").and_then(Value::as_str) {
                out.push_str(&format!(
                    "[binary resource {} ({} bytes base64)]\n",
                    content.get("uri").and_then(Value::as_str).unwrap_or(uri),
                    blob.len()
                ));
            }
        }
        Ok(out.trim_end().to_owned())
    }

    fn list_prompts(&mut self) -> Result<String> {
        let result = self.rpc("prompts/list", json!({}))?;
        let prompts = result
            .get("prompts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if prompts.is_empty() {
            return Ok("(no prompts)".to_owned());
        }
        Ok(prompts
            .iter()
            .map(|prompt| {
                format!(
                    "{}  {}",
                    prompt.get("name").and_then(Value::as_str).unwrap_or(""),
                    prompt
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn get_prompt(&mut self, name: &str, arguments: &Value) -> Result<String> {
        let result = self.rpc("prompts/get", json!({"name": name, "arguments": arguments}))?;
        let messages = result
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(messages
            .iter()
            .map(|message| {
                let role = message
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("user");
                let text = message
                    .get("content")
                    .and_then(|content| content.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                format!("{role}: {text}")
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        match &mut self.transport {
            Transport::Stdio { stdin, stdout, .. } => {
                writeln!(stdin, "{payload}").map_err(BloraError::exec)?;
                stdin.flush().map_err(BloraError::exec)?;
                if method.starts_with("notifications/") {
                    return Ok(Value::Null);
                }
                let mut line = String::new();
                loop {
                    line.clear();
                    let read = stdout.read_line(&mut line).map_err(BloraError::exec)?;
                    if read == 0 {
                        return Err(BloraError::Other(
                            "MCP server closed the connection".to_owned(),
                        ));
                    }
                    let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
                        continue;
                    };
                    if value.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if let Some(error) = value.get("error") {
                        return Err(BloraError::Other(error.to_string()));
                    }
                    return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                }
            }
            Transport::Http {
                url,
                headers,
                session_id,
            } => {
                if method.starts_with("notifications/") {
                    return Ok(Value::Null);
                }
                let mut request = ureq::post(url)
                    .set("Content-Type", "application/json")
                    .set("Accept", "application/json, text/event-stream")
                    .timeout(std::time::Duration::from_secs(self.timeout_secs));
                for (key, value) in headers {
                    request = request.set(key, value);
                }
                if let Some(session) = session_id.as_deref() {
                    request = request.set("Mcp-Session-Id", session);
                }
                let response = request.send_string(&payload.to_string());
                let response = match response {
                    Ok(response) => response,
                    Err(ureq::Error::Status(_, response)) => response,
                    Err(err) => return Err(BloraError::Other(err.to_string())),
                };
                if let Some(session) = response.header("Mcp-Session-Id") {
                    *session_id = Some(session.to_owned());
                }
                let body = response.into_string().map_err(BloraError::exec)?;
                let value = parse_http_body(&body).ok_or_else(|| {
                    BloraError::Other("MCP HTTP response was not JSON".to_owned())
                })?;
                if let Some(error) = value.get("error") {
                    return Err(BloraError::Other(error.to_string()));
                }
                Ok(value.get("result").cloned().unwrap_or(Value::Null))
            }
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Transport::Stdio { child, .. } = &mut self.transport {
            let _ = child.kill();
        }
    }
}

fn parse_http_body(body: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(body.trim()) {
        return Some(value);
    }
    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            if let Ok(value) = serde_json::from_str::<Value>(data.trim()) {
                return Some(value);
            }
        }
    }
    None
}

fn extract_content(result: &Value) -> Option<String> {
    let content = result.get("content")?.as_array()?;
    let text = content
        .iter()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn expand_env(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find('}') {
            let name = &after[..end];
            out.push_str(&std::env::var(name).unwrap_or_default());
            rest = &after[end + 1..];
        } else {
            out.push_str("${");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_environment() {
        let value = expand_env("prefix-${PATH}");
        assert!(value.starts_with("prefix-"));
        assert!(value.len() > "prefix-".len());
        assert_eq!(expand_env("plain"), "plain");
    }

    #[test]
    fn parses_sse_body() {
        let value = parse_http_body("event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1}\n\n");
        assert_eq!(value.unwrap()["id"], 1);
    }

    #[test]
    fn talks_to_the_fake_stdio_server() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fake_mcp_server.py");
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".blora")).unwrap();
        std::fs::write(
            dir.path().join(".blora/mcp.json"),
            serde_json::to_string(&json!({
                "servers": {
                    "default": {
                        "command": "python3",
                        "args": [script.display().to_string()]
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let manager = McpManager::load(dir.path()).unwrap();
        let tools = manager.tool_declarations();
        let echo = tools
            .iter()
            .find(|tool| tool.name == "mcp__echo")
            .expect("echo tool");
        assert!(echo.description.contains("Echo"));
        let result = manager.call("mcp__echo", &json!({"text": "hi"})).unwrap();
        assert_eq!(result, "hi");
        assert!(
            manager
                .list_resources(None)
                .unwrap()
                .contains("fake://readme")
        );
        assert!(
            manager
                .read_resource("default", "fake://readme")
                .unwrap()
                .contains("fake MCP resource")
        );
        assert!(manager.list_prompts(None).unwrap().contains("greet"));
        assert!(
            manager
                .get_prompt("default", "greet", &json!({"name": "Ada"}))
                .unwrap()
                .contains("Hello, Ada!")
        );
    }

    #[test]
    fn loads_configured_servers_and_honours_enabled() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".blora")).unwrap();
        std::fs::write(
            dir.path().join(".blora/mcp.json"),
            r#"{"servers":{"a":{"command":"echo"},"b":{"transport":"http","url":"http://example.test","enabled":false}}}"#,
        )
        .unwrap();
        let manager = McpManager::load(dir.path()).unwrap();
        assert_eq!(manager.servers.len(), 1);
        assert_eq!(manager.servers[0].name, "a");
    }

    #[test]
    fn splits_namespaced_tool_names() {
        let manager = McpManager {
            servers: vec![McpServer {
                name: "github".to_owned(),
                config: McpServerConfig::default(),
                client: Mutex::new(None),
            }],
        };
        let (server, tool) = manager.split_tool_name("mcp__github__list_issues").unwrap();
        assert_eq!(server, "github");
        assert_eq!(tool, "list_issues");
    }
}
