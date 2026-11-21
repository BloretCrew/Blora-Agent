// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Peer {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    buffered: Vec<Value>,
}

/// Test binaries are built without their sibling CLI binaries, so a clean
/// `cargo test --workspace` checkout has no `target/debug/blora` yet. Build it
/// once rather than depending on a previous `cargo build`.
fn cli_binary() -> std::path::PathBuf {
    if let Ok(binary) = std::env::var("BLORA_BIN") {
        return binary.into();
    }
    if let Ok(binary) = std::env::var("CARGO_BIN_EXE_blora") {
        return binary.into();
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate lives under the workspace")
        .to_path_buf();
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace.join(path)
            }
        })
        .unwrap_or_else(|| workspace.join("target"));
    let name = if cfg!(windows) { "blora.exe" } else { "blora" };
    let binary = target.join("debug").join(name);
    if !binary.is_file() {
        static BUILD: std::sync::Once = std::sync::Once::new();
        BUILD.call_once(|| {
            let status = Command::new(env!("CARGO"))
                .args(["build", "--quiet", "-p", "blora-cli", "--bin", "blora"])
                .current_dir(&workspace)
                .status()
                .expect("failed to run cargo build for blora-cli");
            assert!(status.success(), "cargo build for blora-cli failed");
        });
    }
    binary
}

impl Peer {
    fn start() -> Self {
        let binary = cli_binary();
        let mut child = Command::new(binary)
            .args(["sdk", "--stdio", "--allow-test-provider"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            buffered: Vec::new(),
        }
    }

    fn send(&mut self, value: Value) {
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn read(&mut self) -> Value {
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).unwrap();
            assert!(!line.is_empty(), "protocol subprocess exited early");
            let value: Value = serde_json::from_str(&line).unwrap();
            if value.get("method").and_then(Value::as_str) == Some("host_tool.invoke") {
                let params = &value["params"];
                self.send(json!({"jsonrpc":"2.0","id":88,"method":"run.wait","params":{"runId":params["runId"],"timeoutMs":2000}}));
                let result = json!({"parts":[{"type":"text","text":format!("echo:{}", params["arguments"]["text"].as_str().unwrap_or_default())}],"metadata":{}});
                self.send(json!({"jsonrpc":"2.0","id":value["id"],"result":result}));
                continue;
            }
            if value.get("method").and_then(Value::as_str) == Some("host_tool.cancelled") {
                continue;
            }
            return value;
        }
    }

    fn response(&mut self, id: u64) -> Value {
        loop {
            let value = self.read();
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return value;
            }
            self.buffered.push(value);
        }
    }

    fn shutdown(mut self) {
        self.send(json!({"jsonrpc":"2.0","id":99,"method":"agent.shutdown","params":{"cancelRunning":true,"timeoutMs":1000}}));
        let _ = self.response(99);
        drop(self.stdin);
        let _ = self.child.wait();
    }
}

fn initialize(peer: &mut Peer, params: Value) -> Value {
    peer.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":params}));
    peer.response(1)
}

#[test]
fn initialize_requires_explicit_provider_kind() {
    let mut peer = Peer::start();
    let response = initialize(&mut peer, json!({"profile":"web","provider":{}}));
    assert_eq!(response["error"]["data"]["kind"], "invalid_config");
    peer.shutdown();
}

#[test]
fn basic_chat_events_and_repeat_run_wait() {
    let mut peer = Peer::start();
    assert!(initialize(&mut peer, json!({
        "protocolVersion":"1.0","profile":"web",
        "provider":{"kind":"scripted","completions":[{"text":"hello","finish_reason":"stop"}]}
    }))["result"].is_object());
    peer.send(json!({"jsonrpc":"2.0","id":2,"method":"session.create","params":{}}));
    let session = peer.response(2)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    peer.send(json!({"jsonrpc":"2.0","id":3,"method":"run.start","params":{"sessionId":session,"prompt":"hi"}}));
    let run = peer.response(3)["result"]["runId"]
        .as_str()
        .unwrap()
        .to_owned();
    loop {
        let value = peer.read();
        if value["method"] == "run.finished" {
            break;
        }
        peer.buffered.push(value);
    }
    for id in [4, 5] {
        peer.send(json!({"jsonrpc":"2.0","id":id,"method":"run.wait","params":{"runId":run,"timeoutMs":1000}}));
        let response = peer.response(id);
        assert_eq!(response["result"]["status"], "completed", "{response}");
    }
    peer.shutdown();
}

#[test]
fn session_archive_and_subscribe_methods_are_implemented() {
    let mut peer = Peer::start();
    initialize(
        &mut peer,
        json!({"protocolVersion":"1.0","profile":"web","provider":{"kind":"mock"}}),
    );
    peer.send(json!({"jsonrpc":"2.0","id":2,"method":"session.create","params":{}}));
    let session = peer.response(2)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    peer.send(json!({"jsonrpc":"2.0","id":3,"method":"session.subscribe","params":{"sessionId":session,"afterSequence":0}}));
    let sub = peer.response(3)["result"]["subscriptionId"]
        .as_str()
        .unwrap()
        .to_owned();
    peer.send(json!({"jsonrpc":"2.0","id":4,"method":"session.unsubscribe","params":{"subscriptionId":sub}}));
    assert_eq!(peer.response(4)["result"]["unsubscribed"], true);
    peer.send(
        json!({"jsonrpc":"2.0","id":5,"method":"session.archive","params":{"sessionId":session}}),
    );
    assert_eq!(peer.response(5)["result"]["archived"], true);
    peer.shutdown();
}

#[test]
fn host_tool_callback_round_trip() {
    let mut peer = Peer::start();
    initialize(
        &mut peer,
        json!({
            "protocolVersion":"1.0","profile":"web",
            "provider":{"kind":"scripted","completions":[
                {"tool_calls":[{"id":"call_echo","name":"echo","arguments":"{\"text\":\"hi\"}"}],"finish_reason":"tool_calls"},
                {"text":"done","finish_reason":"stop"}
            ]},
            "hostTools":[{"name":"echo","description":"Echo","parameters":{"type":"object","properties":{"text":{"type":"string"}}},"readOnly":true}]
        }),
    );
    peer.send(json!({"jsonrpc":"2.0","id":2,"method":"session.create","params":{}}));
    let session = peer.response(2)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    peer.send(json!({"jsonrpc":"2.0","id":3,"method":"run.start","params":{"sessionId":session,"prompt":"call tool"}}));
    peer.response(3);
    loop {
        if peer.read()["method"] == "run.finished" {
            break;
        }
    }
    peer.shutdown();
}
