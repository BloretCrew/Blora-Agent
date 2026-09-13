// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;

use blora_model::ToolDeclaration;
use blora_types::{BloraError, Result};
use serde_json::{Value, json};

pub struct McpClient {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<std::process::ChildStdout>>,
    next_id: Mutex<u64>,
}

impl McpClient {
    pub fn connect(command: &str) -> Result<Self> {
        let mut parts = command.split_whitespace();
        let program = parts
            .next()
            .ok_or_else(|| BloraError::Other("BLORA_MCP_COMMAND is empty".to_owned()))?;
        let args: Vec<&str> = parts.collect();
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(BloraError::exec)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BloraError::Other("mcp stdin missing".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BloraError::Other("mcp stdout missing".to_owned()))?;
        let client = Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout)),
            next_id: Mutex::new(1),
        };
        let _ = client.rpc(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "blora-agent", "version": "0.1.0"}
            }),
        )?;
        let _ = client.rpc("notifications/initialized", json!({}));
        Ok(client)
    }

    pub fn list_tools(&self) -> Result<Vec<ToolDeclaration>> {
        let result = self.rpc("tools/list", json!({}))?;
        let mut out = Vec::new();
        if let Some(tools) = result.get("tools").and_then(Value::as_array) {
            for tool in tools {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_owned();
                out.push(ToolDeclaration {
                    name: format!("mcp__{name}"),
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("MCP tool")
                        .to_owned(),
                    parameters: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                });
            }
        }
        Ok(out)
    }

    pub fn call(&self, name: &str, arguments: &Value) -> Result<String> {
        let raw = name.strip_prefix("mcp__").unwrap_or(name);
        let result = self.rpc("tools/call", json!({"name": raw, "arguments": arguments}))?;
        Ok(result.to_string())
    }

    fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let id = {
            let mut next = self.next_id.lock().unwrap();
            let id = *next;
            *next += 1;
            id
        };
        let payload = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        {
            let mut stdin = self.stdin.lock().unwrap();
            writeln!(stdin, "{payload}").map_err(BloraError::exec)?;
            stdin.flush().map_err(BloraError::exec)?;
        }
        if method.starts_with("notifications/") {
            return Ok(Value::Null);
        }
        let mut stdout = self.stdout.lock().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).map_err(BloraError::exec)?;
        let value: Value =
            serde_json::from_str(line.trim()).map_err(|err| BloraError::event(err.to_string()))?;
        if let Some(error) = value.get("error") {
            return Err(BloraError::Other(error.to_string()));
        }
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.kill();
        }
    }
}
