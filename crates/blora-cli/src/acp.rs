// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::io::{BufRead, Write};

use blora_runtime::{CancelToken, RunOptions, Runtime};
use blora_storage::CreateSession;
use blora_types::Mode;
use serde_json::{Value, json};

pub fn serve(
    runtime: &Runtime,
    workspace: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut cancel = CancelToken::new();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = serde_json::from_str(&line)?;
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => {
                json!({"protocolVersion": "0.1.0", "serverInfo": {"name": "blora-agent"}})
            }
            "session/new" => {
                let cwd = params
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| workspace.display().to_string());
                let session = runtime.create_session(CreateSession {
                    title: Some("acp".to_owned()),
                    workspace_path: cwd,
                    mode: Mode::Code,
                    parent_session_id: None,
                })?;
                json!({"sessionId": session.to_string()})
            }
            "session/prompt" => {
                let session = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .ok_or("missing sessionId")?;
                let prompt = params.get("prompt").and_then(Value::as_str).unwrap_or("");
                cancel = CancelToken::new();
                let run = runtime.run(
                    &session.parse()?,
                    prompt,
                    &cancel,
                    &RunOptions {
                        mock: std::env::var("BLORA_API_KEY").is_err()
                            && std::env::var("OPENAI_API_KEY").is_err()
                            && std::env::var("ANTHROPIC_API_KEY").is_err()
                            && std::env::var("GEMINI_API_KEY").is_err(),
                        ..RunOptions::default()
                    },
                )?;
                json!({"runId": run.to_string()})
            }
            "session/cancel" => {
                cancel.cancel();
                json!({"cancelled": true})
            }
            "shutdown" => {
                cancel.cancel();
                let reply = json!({"jsonrpc": "2.0", "id": id, "result": {}});
                writeln!(stdout, "{reply}")?;
                break;
            }
            other => json!({"error": format!("unknown method {other}")}),
        };
        let reply = json!({"jsonrpc": "2.0", "id": id, "result": result});
        writeln!(stdout, "{reply}")?;
        stdout.flush()?;
    }
    Ok(())
}
