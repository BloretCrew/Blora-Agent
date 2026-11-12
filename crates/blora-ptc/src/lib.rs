// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Programmatic tool calling: run Python or JavaScript with a loopback bridge
//! back into Blora's tool registry.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use blora_types::{BloraError, Result};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct PtcRequest {
    pub language: String,
    pub code: String,
    pub timeout: Duration,
    pub max_tool_calls: usize,
}

#[derive(Clone, Debug, Default)]
pub struct PtcOutput {
    pub stdout: String,
    pub stderr: String,
    pub value: Option<Value>,
    pub logs: Vec<String>,
    pub tool_calls: usize,
}

const DENIED_TOOLS: &[&str] = &[
    "execute_code",
    "subagent",
    "delegate",
    "ask_user",
    "exit_plan_mode",
    "artifact",
    "schedule",
    "schedule_task",
    "team",
    "team_task",
    "goal",
    "remember",
    "forget",
    "skill_manage",
    "handoff",
];

/// Run user code and let it call allowed Blora tools through `call_tool`.
pub fn run<F>(request: &PtcRequest, workspace: &Path, mut call_tool: F) -> Result<PtcOutput>
where
    F: FnMut(&str, &Value) -> Result<String>,
{
    let language = request.language.to_ascii_lowercase();
    let (extension, program, args) = match language.as_str() {
        "python" | "python3" => ("py", "python3", vec!["-I", "-u"]),
        "javascript" | "js" | "node" => ("js", "node", Vec::new()),
        other => {
            return Err(BloraError::Other(format!(
                "unsupported execute_code language: {other}"
            )));
        }
    };
    let run_dir = workspace.join(".blora/run");
    fs::create_dir_all(&run_dir).map_err(BloraError::exec)?;
    let id = Uuid::now_v7().simple();
    let script = run_dir.join(format!("ptc_{id}.{extension}"));
    let source = if language.starts_with("python") {
        python_bootstrap(&request.code)
    } else {
        javascript_bootstrap(&request.code)
    };
    fs::write(&script, source).map_err(BloraError::exec)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(BloraError::exec)?;
    let port = listener.local_addr().map_err(BloraError::exec)?.port();
    let token = Uuid::now_v7().simple().to_string();
    let mut command = Command::new(program);
    command
        .args(&args)
        .arg(&script)
        .current_dir(workspace)
        .env("BLORA_PTC_PORT", port.to_string())
        .env("BLORA_PTC_TOKEN", &token)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| BloraError::Exec(format!("failed to start {program}: {err}")))?;
    let stdout = Arc::new(Mutex::new(String::new()));
    let stderr = Arc::new(Mutex::new(String::new()));
    if let Some(out) = child.stdout.take() {
        spawn_reader(out, stdout.clone());
    }
    if let Some(err) = child.stderr.take() {
        spawn_reader(err, stderr.clone());
    }
    let deadline = Instant::now() + request.timeout;
    listener.set_nonblocking(true).map_err(BloraError::exec)?;
    let stream = match accept_connection(&listener, &mut child, deadline) {
        Ok(stream) => stream,
        Err(err) => {
            let _ = child.kill();
            let _ = fs::remove_file(&script);
            return Err(err);
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(BloraError::exec)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(BloraError::exec)?);
    let mut writer = stream;
    let mut line = String::new();
    reader.read_line(&mut line).map_err(BloraError::exec)?;
    let hello: Value = serde_json::from_str(line.trim())
        .map_err(|_| BloraError::Other("PTC bridge sent an invalid hello".to_owned()))?;
    if hello.get("method").and_then(Value::as_str) != Some("hello")
        || hello.get("token").and_then(Value::as_str) != Some(token.as_str())
    {
        let _ = child.kill();
        let _ = fs::remove_file(&script);
        return Err(BloraError::Policy(
            "PTC bridge authentication failed".to_owned(),
        ));
    }
    writeln!(writer, "{}", json!({"ok": true})).map_err(BloraError::exec)?;
    writer.flush().map_err(BloraError::exec)?;

    let mut tool_calls = 0usize;
    let mut value = None;
    let mut logs = Vec::new();
    loop {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = fs::remove_file(&script);
            return Err(BloraError::Other("execute_code timed out".to_owned()));
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                continue;
            }
            Err(err) => {
                let _ = child.kill();
                let _ = fs::remove_file(&script);
                return Err(BloraError::exec(err));
            }
        }
        let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        match method {
            "call" => {
                tool_calls += 1;
                let tool = message
                    .get("params")
                    .and_then(|params| params.get("tool"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = message
                    .get("params")
                    .and_then(|params| params.get("arguments"))
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let response = if tool_calls > request.max_tool_calls {
                    json!({"id": id, "ok": false, "error": "tool call budget exceeded"})
                } else if !allowed(tool) {
                    json!({"id": id, "ok": false, "error": format!("tool not allowed: {tool}")})
                } else {
                    match call_tool(tool, &arguments) {
                        Ok(result) => json!({"id": id, "ok": true, "result": result}),
                        Err(err) => json!({"id": id, "ok": false, "error": err.to_string()}),
                    }
                };
                writeln!(writer, "{response}").map_err(BloraError::exec)?;
                writer.flush().map_err(BloraError::exec)?;
            }
            "log" => {
                if let Some(text) = message
                    .get("params")
                    .and_then(|params| params.get("text"))
                    .and_then(Value::as_str)
                {
                    logs.push(text.to_owned());
                }
            }
            "finish" => {
                value = message
                    .get("params")
                    .and_then(|params| params.get("value"))
                    .cloned();
                break;
            }
            _ => {}
        }
    }
    drop(reader);
    drop(writer);
    wait_with_timeout(&mut child, Duration::from_secs(2));
    let _ = fs::remove_file(&script);
    Ok(PtcOutput {
        stdout: stdout.lock().map(|text| text.clone()).unwrap_or_default(),
        stderr: stderr.lock().map(|text| text.clone()).unwrap_or_default(),
        value,
        logs,
        tool_calls,
    })
}

#[must_use]
pub fn allowed(tool: &str) -> bool {
    !DENIED_TOOLS.contains(&tool) && !tool.starts_with("mcp__") && !tool.starts_with("plugin__")
}

fn accept_connection(
    listener: &TcpListener,
    child: &mut Child,
    deadline: Instant,
) -> Result<TcpStream> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(BloraError::Other(
                        "execute_code timed out waiting for the bridge".to_owned(),
                    ));
                }
                if child.try_wait().ok().flatten().is_some() {
                    return Err(BloraError::Other(
                        "execute_code process exited before connecting".to_owned(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) => return Err(BloraError::exec(err)),
        }
    }
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn spawn_reader(mut reader: impl Read + Send + 'static, target: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if let Ok(mut text) = target.lock() {
                        text.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn python_bootstrap(code: &str) -> String {
    format!(
        r#"import json, os, socket
_sock = socket.create_connection(("127.0.0.1", int(os.environ["BLORA_PTC_PORT"])), timeout=30)
_f = _sock.makefile("rwb")
_next_id = 0
def _send(message):
    _f.write((json.dumps(message) + "\n").encode("utf-8"))
    _f.flush()
_send({{"method": "hello", "token": os.environ["BLORA_PTC_TOKEN"]}})
_f.readline()
def call_tool(name, arguments=None):
    global _next_id
    _next_id += 1
    _send({{"id": _next_id, "method": "call", "params": {{"tool": name, "arguments": arguments or {{}}}}}})
    line = _f.readline()
    if not line:
        raise RuntimeError("Blora tool bridge closed")
    response = json.loads(line)
    if not response.get("ok"):
        raise RuntimeError(response.get("error", "tool failed"))
    return response.get("result")
def finish(value=None):
    _send({{"method": "finish", "params": {{"value": value}}}})
def log(text):
    _send({{"method": "log", "params": {{"text": str(text)}}}})

{code}
"#
    )
}

fn javascript_bootstrap(code: &str) -> String {
    format!(
        r#"const net = require("net");
const sock = net.createConnection({{ host: "127.0.0.1", port: Number(process.env.BLORA_PTC_PORT) }});
let buffer = "";
let nextId = 0;
const pending = new Map();
sock.on("data", (chunk) => {{
  buffer += chunk.toString("utf8");
  let index;
  while ((index = buffer.indexOf("\n")) >= 0) {{
    const line = buffer.slice(0, index);
    buffer = buffer.slice(index + 1);
    if (!line.trim()) continue;
    const message = JSON.parse(line);
    if (message.id && pending.has(message.id)) {{
      const request = pending.get(message.id);
      pending.delete(message.id);
      if (message.ok) request.resolve(message.result);
      else request.reject(new Error(message.error || "tool failed"));
    }}
  }}
}});
function send(message) {{ sock.write(JSON.stringify(message) + "\n"); }}
function callTool(name, arguments = {{}}) {{
  return new Promise((resolve, reject) => {{
    const id = ++nextId;
    pending.set(id, {{ resolve, reject }});
    send({{ id, method: "call", params: {{ tool: name, arguments }} }});
  }});
}}
function finish(value = null) {{ send({{ method: "finish", params: {{ value }} }}); }}
function log(text) {{ send({{ method: "log", params: {{ text: String(text) }} }}); }}
send({{ method: "hello", token: process.env.BLORA_PTC_TOKEN }});
(async () => {{
{code}
}})().catch((error) => {{ log(error && error.stack ? error.stack : String(error)); process.exitCode = 1; }});
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_can_call_tools() {
        if Command::new("python3").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("data.txt"), "hello").unwrap();
        let request = PtcRequest {
            language: "python".to_owned(),
            code: "text = call_tool('read_file', {'path': 'data.txt'})\nfinish(text)".to_owned(),
            timeout: Duration::from_secs(10),
            max_tool_calls: 5,
        };
        let output = run(&request, dir.path(), |tool, arguments| {
            assert_eq!(tool, "read_file");
            assert_eq!(arguments["path"], "data.txt");
            Ok("hello".to_owned())
        })
        .unwrap();
        assert_eq!(output.value, Some(json!("hello")));
        assert_eq!(output.tool_calls, 1);
    }

    #[test]
    fn javascript_can_call_tools() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let request = PtcRequest {
            language: "javascript".to_owned(),
            code: "const text = await callTool('read_file', {path: 'data.txt'});\nfinish(text);"
                .to_owned(),
            timeout: Duration::from_secs(10),
            max_tool_calls: 5,
        };
        let output = run(&request, dir.path(), |_, _| Ok("hello".to_owned())).unwrap();
        assert_eq!(output.value, Some(json!("hello")));
    }
}
