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
                        "command": {"type": "string"},
                        "background": {"type": "boolean"}
                    },
                    "required": ["command"]
                }),
            },
            ToolSpec {
                name: "schedule_task",
                description: "Queue a durable background task on this session. delay_seconds=0 runs on the next scheduler tick. Optional 5-field cron.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"},
                        "prompt": {"type": "string"},
                        "delay_seconds": {"type": "integer", "minimum": 0},
                        "cron": {"type": "string", "description": "5-field cron, for example 0 * * * *"}
                    },
                    "required": ["title", "prompt"]
                }),
            },
            ToolSpec {
                name: "git_status",
                description: "Show git status for the workspace.",
                parameters: json!({"type": "object", "properties": {}}),
            },
            ToolSpec {
                name: "git_diff",
                description: "Show git diff --stat against HEAD.",
                parameters: json!({"type": "object", "properties": {}}),
            },
            ToolSpec {
                name: "git_log",
                description: "Show recent git commits.",
                parameters: json!({"type": "object", "properties": {}}),
            },
            ToolSpec {
                name: "git_branch",
                description: "Show git branches for the workspace.",
                parameters: json!({"type": "object", "properties": {}}),
            },
            ToolSpec {
                name: "git_worktree",
                description: "List or add a git worktree inside the workspace.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["list", "add"]},
                        "path": {"type": "string"}
                    }
                }),
            },
            ToolSpec {
                name: "process",
                description: "List or kill background processes started by Blora.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["list", "kill"]},
                        "pid": {"type": "integer"}
                    },
                    "required": ["action"]
                }),
            },
            ToolSpec {
                name: "apply_patch",
                description: "Replace old_string with new_string in a workspace file.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "old_string": {"type": "string"},
                        "new_string": {"type": "string"}
                    },
                    "required": ["path", "old_string", "new_string"]
                }),
            },
            ToolSpec {
                name: "delegate",
                description: "Spawn a child agent with a limited turn budget. Roles: research, review, plan (read-only), code.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "role": {"type": "string"},
                        "prompt": {"type": "string"}
                    },
                    "required": ["prompt"]
                }),
            },
            ToolSpec {
                name: "remember",
                description: "Store an explicit memory for this workspace.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "key": {"type": "string"},
                        "value": {"type": "string"}
                    },
                    "required": ["key", "value"]
                }),
            },
            ToolSpec {
                name: "recall",
                description: "Read stored memories. Omit key to list all.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "key": {"type": "string"}
                    }
                }),
            },
            ToolSpec {
                name: "forget",
                description: "Delete a stored memory by key.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "key": {"type": "string"}
                    },
                    "required": ["key"]
                }),
            },
            ToolSpec {
                name: "handoff",
                description: "Return a structured summary to the parent agent.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "summary": {"type": "string"},
                        "files": {"type": "string"}
                    },
                    "required": ["summary"]
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
            "shell" => {
                let command = required_str(arguments, "command")?;
                if arguments
                    .get("background")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    backend.shell_background(command)
                } else {
                    backend.shell(command)
                }
            }
            "git_status" => backend.git_status(),
            "git_diff" => backend.git_diff(),
            "git_log" => backend.git_log(),
            "git_branch" => backend.git_branch(),
            "git_worktree" => backend.git_worktree(
                optional_str(arguments, "action").unwrap_or("list"),
                optional_str(arguments, "path"),
            ),
            "process" => match optional_str(arguments, "action").unwrap_or("list") {
                "kill" => {
                    let pid = arguments
                        .get("pid")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| BloraError::Other("process kill needs pid".to_owned()))?;
                    backend.process_kill(u32::try_from(pid).unwrap_or(0))
                }
                _ => backend.process_list(),
            },
            "handoff" => {
                let summary = required_str(arguments, "summary")?;
                let files = optional_str(arguments, "files").unwrap_or("");
                Ok(format!("HANDOFF\n{summary}\n{files}"))
            }
            "apply_patch" => {
                backend.apply_patch(
                    required_str(arguments, "path")?,
                    required_str(arguments, "old_string")?,
                    required_str(arguments, "new_string")?,
                )?;
                Ok("patched file".to_owned())
            }
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
