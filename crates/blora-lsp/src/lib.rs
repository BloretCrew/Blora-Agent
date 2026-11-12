// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Minimal LSP client for code intelligence actions.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use blora_types::{BloraError, Result};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LspConfig {
    #[serde(default)]
    pub servers: BTreeMap<String, LspServerConfig>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LspServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    pub language_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct LspRequest {
    pub action: String,
    pub path: String,
    pub line: Option<u32>,
    pub character: Option<u32>,
    pub query: Option<String>,
    pub timeout_secs: Option<u64>,
}

struct Resolved {
    name: String,
    config: LspServerConfig,
    root: PathBuf,
    file: PathBuf,
    uri: String,
    language_id: String,
}

fn clients() -> &'static Mutex<BTreeMap<String, LspClient>> {
    static CLIENTS: OnceLock<Mutex<BTreeMap<String, LspClient>>> = OnceLock::new();
    CLIENTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub fn server_for(workspace: &Path, path: &str) -> Result<String> {
    Ok(resolve(workspace, path)?.name)
}

pub fn action(workspace: &Path, request: &LspRequest) -> Result<String> {
    let resolved = resolve(workspace, &request.path)?;
    let key = format!("{}@{}", resolved.name, resolved.root.display());
    let mut clients = clients()
        .lock()
        .map_err(|_| BloraError::Other("LSP client lock poisoned".to_owned()))?;
    if !clients.contains_key(&key) {
        let client = LspClient::connect(&resolved.config, &resolved.root)?;
        clients.insert(key.clone(), client);
    }
    let client = clients
        .get_mut(&key)
        .ok_or_else(|| BloraError::Other("LSP client missing".to_owned()))?;
    let timeout = request
        .timeout_secs
        .or(resolved.config.timeout_secs)
        .unwrap_or(10);
    client.request(
        &resolved,
        &request.action,
        request.line,
        request.character,
        request.query.as_deref(),
        Duration::from_secs(timeout.clamp(1, 120)),
    )
}

fn resolve(workspace: &Path, path: &str) -> Result<Resolved> {
    let file = workspace.join(path);
    let file = file
        .canonicalize()
        .map_err(|err| BloraError::Exec(format!("LSP path: {err}")))?;
    let root_workspace = workspace
        .canonicalize()
        .map_err(|err| BloraError::Exec(format!("LSP workspace: {err}")))?;
    if !file.starts_with(&root_workspace) {
        return Err(BloraError::Policy("LSP path escapes workspace".to_owned()));
    }
    let extension = file
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let config = load_config(workspace);
    let (name, server, language_id) = config
        .servers
        .iter()
        .find_map(|(name, server)| {
            server
                .language_ids
                .get(&extension)
                .map(|language| (name.clone(), server.clone(), language.clone()))
        })
        .ok_or_else(|| {
            BloraError::Other(format!(
                "no language server configured for .{extension}; add .blora/lsp.json"
            ))
        })?;
    let root = find_root(&server, &file, &root_workspace);
    let uri = url::Url::from_file_path(&file)
        .map_err(|()| BloraError::Exec("invalid LSP path".to_owned()))?
        .to_string();
    Ok(Resolved {
        name,
        config: server,
        root,
        file,
        uri,
        language_id,
    })
}

fn find_root(server: &LspServerConfig, file: &Path, workspace: &Path) -> PathBuf {
    let mut current = file.parent();
    while let Some(dir) = current {
        if !dir.starts_with(workspace) {
            break;
        }
        if server
            .root_markers
            .iter()
            .any(|marker| dir.join(marker).exists())
        {
            return dir.to_path_buf();
        }
        if dir == workspace {
            break;
        }
        current = dir.parent();
    }
    workspace.to_path_buf()
}

fn load_config(workspace: &Path) -> LspConfig {
    let mut config = LspConfig {
        servers: default_servers(),
    };
    for path in config_paths(workspace) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(overrides) = serde_json::from_str::<LspConfig>(&text) else {
            tracing_warn(&format!("invalid LSP config: {}", path.display()));
            continue;
        };
        for (name, server) in overrides.servers {
            config.servers.insert(name, server);
        }
    }
    config
}

fn tracing_warn(message: &str) {
    eprintln!("{message}");
}

fn config_paths(workspace: &Path) -> Vec<PathBuf> {
    let mut paths = vec![workspace.join(".blora/lsp.json")];
    let home = std::env::var("BLORA_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".blora")));
    if let Some(home) = home {
        paths.push(home.join("lsp.json"));
    }
    paths
}

fn default_servers() -> BTreeMap<String, LspServerConfig> {
    let mut servers = BTreeMap::new();
    servers.insert(
        "rust".to_owned(),
        LspServerConfig {
            command: "rust-analyzer".to_owned(),
            root_markers: vec!["Cargo.toml".to_owned()],
            language_ids: BTreeMap::from([("rs".to_owned(), "rust".to_owned())]),
            ..LspServerConfig::default()
        },
    );
    servers.insert(
        "typescript".to_owned(),
        LspServerConfig {
            command: "typescript-language-server".to_owned(),
            args: vec!["--stdio".to_owned()],
            root_markers: vec![
                "package.json".to_owned(),
                "tsconfig.json".to_owned(),
                "jsconfig.json".to_owned(),
            ],
            language_ids: BTreeMap::from([
                ("ts".to_owned(), "typescript".to_owned()),
                ("tsx".to_owned(), "typescriptreact".to_owned()),
                ("js".to_owned(), "javascript".to_owned()),
                ("jsx".to_owned(), "javascriptreact".to_owned()),
            ]),
            ..LspServerConfig::default()
        },
    );
    servers.insert(
        "python".to_owned(),
        LspServerConfig {
            command: "pyright-langserver".to_owned(),
            args: vec!["--stdio".to_owned()],
            root_markers: vec![
                "pyproject.toml".to_owned(),
                "setup.py".to_owned(),
                "requirements.txt".to_owned(),
            ],
            language_ids: BTreeMap::from([("py".to_owned(), "python".to_owned())]),
            ..LspServerConfig::default()
        },
    );
    servers.insert(
        "go".to_owned(),
        LspServerConfig {
            command: "gopls".to_owned(),
            root_markers: vec!["go.mod".to_owned()],
            language_ids: BTreeMap::from([("go".to_owned(), "go".to_owned())]),
            ..LspServerConfig::default()
        },
    );
    servers.insert(
        "clangd".to_owned(),
        LspServerConfig {
            command: "clangd".to_owned(),
            root_markers: vec![
                "compile_commands.json".to_owned(),
                "CMakeLists.txt".to_owned(),
            ],
            language_ids: BTreeMap::from([
                ("c".to_owned(), "c".to_owned()),
                ("h".to_owned(), "c".to_owned()),
                ("cc".to_owned(), "cpp".to_owned()),
                ("cpp".to_owned(), "cpp".to_owned()),
                ("hpp".to_owned(), "cpp".to_owned()),
            ]),
            ..LspServerConfig::default()
        },
    );
    servers
}

struct LspClient {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: u64,
    opened: BTreeSet<String>,
    diagnostics: BTreeMap<String, Vec<Value>>,
}

impl LspClient {
    fn connect(config: &LspServerConfig, root: &Path) -> Result<Self> {
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .envs(&config.env)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| {
                BloraError::Exec(format!("failed to start {}: {err}", config.command))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BloraError::Other("LSP stdin missing".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BloraError::Other("LSP stdout missing".to_owned()))?;
        let (tx, rx) = channel();
        std::thread::spawn(move || reader_loop(stdout, tx));
        let mut client = Self {
            child,
            stdin,
            rx,
            next_id: 1,
            opened: BTreeSet::new(),
            diagnostics: BTreeMap::new(),
        };
        let root_uri = url::Url::from_directory_path(root)
            .map_err(|()| BloraError::Exec("invalid LSP root".to_owned()))?
            .to_string();
        let _ = client.rpc(
            "initialize",
            json!({
                "processId": null,
                "rootUri": root_uri,
                "workspaceFolders": [{"uri": root_uri, "name": "workspace"}],
                "capabilities": {
                    "textDocument": {
                        "hover": {"contentFormat": ["markdown", "plaintext"]},
                        "definition": {},
                        "references": {},
                        "documentSymbol": {},
                        "publishDiagnostics": {}
                    },
                    "workspace": {"symbol": {}}
                }
            }),
            Duration::from_secs(15),
        )?;
        client.notify("initialized", json!({}))?;
        Ok(client)
    }

    fn request(
        &mut self,
        resolved: &Resolved,
        action: &str,
        line: Option<u32>,
        character: Option<u32>,
        query: Option<&str>,
        timeout: Duration,
    ) -> Result<String> {
        self.did_open(resolved)?;
        let line = line.unwrap_or(1).saturating_sub(1);
        let character = character.unwrap_or(1).saturating_sub(1);
        let position = json!({"line": line, "character": character});
        let text_document = json!({"uri": resolved.uri});
        match action {
            "diagnostics" => self.wait_diagnostics(&resolved.uri, timeout),
            "hover" => {
                let result = self.rpc(
                    "textDocument/hover",
                    json!({"textDocument": text_document, "position": position}),
                    timeout,
                )?;
                Ok(format_hover(&result))
            }
            "definition" => {
                let result = self.rpc(
                    "textDocument/definition",
                    json!({"textDocument": text_document, "position": position}),
                    timeout,
                )?;
                Ok(format_locations(&result))
            }
            "references" => {
                let result = self.rpc(
                    "textDocument/references",
                    json!({
                        "textDocument": text_document,
                        "position": position,
                        "context": {"includeDeclaration": true}
                    }),
                    timeout,
                )?;
                Ok(format_locations(&result))
            }
            "symbols" => {
                let result = self.rpc(
                    "textDocument/documentSymbol",
                    json!({"textDocument": text_document}),
                    timeout,
                )?;
                Ok(format_symbols(&result, 0))
            }
            "workspace_symbols" => {
                let result = self.rpc(
                    "workspace/symbol",
                    json!({"query": query.unwrap_or_default()}),
                    timeout,
                )?;
                Ok(format_symbols(&result, 0))
            }
            other => Err(BloraError::Other(format!("unknown LSP action: {other}"))),
        }
    }

    fn did_open(&mut self, resolved: &Resolved) -> Result<()> {
        if !self.opened.insert(resolved.uri.clone()) {
            return Ok(());
        }
        let text = std::fs::read_to_string(&resolved.file).map_err(BloraError::exec)?;
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": resolved.uri,
                    "languageId": resolved.language_id,
                    "version": 1,
                    "text": text
                }
            }),
        )
    }

    fn wait_diagnostics(&mut self, uri: &str, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(diagnostics) = self.diagnostics.remove(uri) {
                return Ok(format_diagnostics(&diagnostics));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(remaining) {
                Ok(value) => self.absorb(value),
                Err(RecvTimeoutError::Timeout) => return Ok("no diagnostics".to_owned()),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(BloraError::Other("LSP server closed".to_owned()));
                }
            }
        }
    }

    fn rpc(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send(&payload)?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(remaining) {
                Ok(value) => {
                    if value.get("id").and_then(Value::as_u64) == Some(id) {
                        if let Some(error) = value.get("error") {
                            return Err(BloraError::Other(error.to_string()));
                        }
                        return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                    }
                    self.absorb(value);
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(BloraError::Other(format!("LSP {method} timed out")));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(BloraError::Other("LSP server closed".to_owned()));
                }
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn send(&mut self, payload: &Value) -> Result<()> {
        let body = payload.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len())
            .map_err(BloraError::exec)?;
        self.stdin.flush().map_err(BloraError::exec)
    }

    fn absorb(&mut self, value: Value) {
        if value.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics") {
            if let Some(params) = value.get("params") {
                if let Some(uri) = params.get("uri").and_then(Value::as_str) {
                    let diagnostics = params
                        .get("diagnostics")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    self.diagnostics.insert(uri.to_owned(), diagnostics);
                }
            }
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.notify("shutdown", json!({}));
        let _ = self.notify("exit", json!({}));
        let _ = self.child.kill();
    }
}

fn reader_loop(stdout: impl Read + Send + 'static, tx: Sender<Value>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
        if length == 0 {
            continue;
        }
        let mut buffer = vec![0u8; length];
        if reader.read_exact(&mut buffer).is_err() {
            return;
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&buffer) {
            if tx.send(value).is_err() {
                return;
            }
        }
    }
}

fn format_hover(value: &Value) -> String {
    let contents = value.get("contents").unwrap_or(value);
    match contents {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(format_marked)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => format_marked(contents),
    }
}

fn format_marked(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Object(map) => map
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

fn format_locations(value: &Value) -> String {
    let items = match value {
        Value::Array(items) => items.clone(),
        Value::Null => return "no location".to_owned(),
        other => vec![other.clone()],
    };
    if items.is_empty() {
        return "no location".to_owned();
    }
    items
        .iter()
        .filter_map(|item| {
            let uri = item
                .get("uri")
                .or_else(|| item.get("targetUri"))
                .and_then(Value::as_str)?;
            let range = item
                .get("range")
                .or_else(|| item.get("targetSelectionRange"))
                .or_else(|| item.get("targetRange"))?;
            let line = range
                .get("start")
                .and_then(|start| start.get("line"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            let character = range
                .get("start")
                .and_then(|start| start.get("character"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            Some(format!("{uri}:{line}:{character}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_symbols(value: &Value, depth: usize) -> String {
    let items = match value {
        Value::Array(items) => items.clone(),
        Value::Null => return "no symbols".to_owned(),
        other => vec![other.clone()],
    };
    let mut out = Vec::new();
    for item in items {
        let name = item.get("name").and_then(Value::as_str).unwrap_or("symbol");
        let kind = item.get("kind").and_then(Value::as_u64).unwrap_or(0);
        let line = item
            .get("range")
            .or_else(|| {
                item.get("location")
                    .and_then(|location| location.get("range"))
            })
            .and_then(|range| range.get("start"))
            .and_then(|start| start.get("line"))
            .and_then(Value::as_u64)
            .map(|line| line + 1);
        out.push(format!(
            "{}{} (kind {kind}){}",
            "  ".repeat(depth),
            name,
            line.map(|line| format!(" line {line}")).unwrap_or_default()
        ));
        if let Some(children) = item.get("children").and_then(Value::as_array) {
            out.push(format_symbols(&Value::Array(children.clone()), depth + 1));
        }
    }
    out.join("\n")
}

fn format_diagnostics(diagnostics: &[Value]) -> String {
    if diagnostics.is_empty() {
        return "no diagnostics".to_owned();
    }
    diagnostics
        .iter()
        .map(|diagnostic| {
            let line = diagnostic
                .get("range")
                .and_then(|range| range.get("start"))
                .and_then(|start| start.get("line"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            let severity = match diagnostic
                .get("severity")
                .and_then(Value::as_u64)
                .unwrap_or(0)
            {
                1 => "error",
                2 => "warning",
                3 => "info",
                _ => "hint",
            };
            format!(
                "{line}: {severity}: {}",
                diagnostic
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_servers_cover_common_languages() {
        let servers = default_servers();
        assert!(servers["rust"].language_ids.contains_key("rs"));
        assert!(servers["typescript"].language_ids.contains_key("tsx"));
        assert!(servers["python"].language_ids.contains_key("py"));
    }

    #[test]
    fn formats_locations_and_diagnostics() {
        let value =
            json!([{"uri": "file:///a.rs", "range": {"start": {"line": 2, "character": 4}}}]);
        assert_eq!(format_locations(&value), "file:///a.rs:3:5");
        let diagnostics =
            json!([{"range": {"start": {"line": 0}}, "severity": 1, "message": "bad"}]);
        assert_eq!(
            format_diagnostics(diagnostics.as_array().unwrap()),
            "1: error: bad"
        );
    }

    #[test]
    fn talks_to_fake_lsp_server() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fake_lsp_server.py");
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".blora")).unwrap();
        std::fs::write(
            dir.path().join(".blora/lsp.json"),
            serde_json::to_string(&json!({
                "servers": {
                    "rust": {
                        "command": "python3",
                        "args": [script.display().to_string()],
                        "root_markers": ["."],
                        "language_ids": {"rs": "rust"},
                        "timeout_secs": 5
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        let request = |action: &str| LspRequest {
            action: action.to_owned(),
            path: "main.rs".to_owned(),
            line: Some(1),
            character: Some(1),
            query: Some("main".to_owned()),
            timeout_secs: Some(5),
        };
        let diagnostics = action(dir.path(), &request("diagnostics")).unwrap();
        assert!(diagnostics.contains("fake diagnostic"), "{diagnostics}");
        let hover = action(dir.path(), &request("hover")).unwrap();
        assert!(hover.contains("fake hover"), "{hover}");
        let definition = action(dir.path(), &request("definition")).unwrap();
        assert!(definition.contains(":1:1"), "{definition}");
        let symbols = action(dir.path(), &request("symbols")).unwrap();
        assert!(symbols.contains("main"), "{symbols}");
    }
}
