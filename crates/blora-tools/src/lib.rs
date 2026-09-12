// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Built-in tools. Schemas are provider-facing; execution stays in-process.

use blora_exec::LocalBackend;
use blora_types::{BloraError, Result};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Clone, Debug)]
pub struct ToolRegistry;

impl ToolRegistry {
    #[must_use]
    pub fn specs() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "read_file",
                description: "Read a UTF-8 text file inside the workspace.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path relative to the workspace"}
                    },
                    "required": ["path"]
                }),
            },
            ToolSpec {
                name: "write_file",
                description: "Write a UTF-8 text file inside the workspace.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "contents": {"type": "string"}
                    },
                    "required": ["path", "contents"]
                }),
            },
            ToolSpec {
                name: "list_dir",
                description: "List files and directories inside the workspace.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "default": "."}
                    }
                }),
            },
            ToolSpec {
                name: "search",
                description: "Search workspace files with a regular expression.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string"},
                        "path": {"type": "string"}
                    },
                    "required": ["pattern"]
                }),
            },
            ToolSpec {
                name: "shell",
                description: "Run a shell command in the workspace. Requires approval unless auto-approve is enabled.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "command": {"type": "string"}
                    },
                    "required": ["command"]
                }),
            },
            ToolSpec {
                name: "schedule_task",
                description: "Queue a durable background task on this session. delay_seconds=0 runs on the next scheduler tick.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"},
                        "prompt": {"type": "string"},
                        "delay_seconds": {"type": "integer", "minimum": 0}
                    },
                    "required": ["title", "prompt"]
                }),
            },
            ToolSpec {
                name: "delegate",
                description: "Spawn a child agent with a limited turn budget. The child runs in Code mode and returns a summary.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "role": {"type": "string"},
                        "prompt": {"type": "string"}
                    },
                    "required": ["prompt"]
                }),
            },
        ]
    }

    pub fn execute(backend: &LocalBackend, name: &str, arguments: &Value) -> Result<String> {
        match name {
            "read_file" => backend.read_file(required_str(arguments, "path")?),
            "write_file" => {
                backend.write_file(
                    required_str(arguments, "path")?,
                    required_str(arguments, "contents")?,
                )?;
                Ok("wrote file".to_owned())
            }
            "list_dir" => backend.list_dir(optional_str(arguments, "path").unwrap_or(".")),
            "search" => backend.search(
                required_str(arguments, "pattern")?,
                optional_str(arguments, "path"),
            ),
            "shell" => backend.shell(required_str(arguments, "command")?),
            other => Err(BloraError::Other(format!("unknown tool: {other}"))),
        }
    }
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| BloraError::Other(format!("missing string field {key}")))
}

fn optional_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}
