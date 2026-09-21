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
    /// Read-only tools can run concurrently and are allowed in read-only roles.
    pub read_only: bool,
}

#[derive(Clone, Debug)]
pub struct ToolRegistry;

impl ToolRegistry {
    #[must_use]
    pub fn specs() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "read_file",
                description: "Read a UTF-8 text file inside the workspace. Optional offset/limit select a line range for large files.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path relative to the workspace"},
                        "offset": {"type": "integer", "minimum": 1, "description": "First line to return (1-based)"},
                        "limit": {"type": "integer", "minimum": 1, "description": "Maximum number of lines"}
                    },
                    "required": ["path"]
                }),
                read_only: true,
            },
            ToolSpec {
                name: "write_file",
                description: "Create or fully replace a UTF-8 text file inside the workspace. Prefer apply_patch for edits to existing files.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "contents": {"type": "string"}
                    },
                    "required": ["path", "contents"]
                }),
                read_only: false,
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
                read_only: true,
            },
            ToolSpec {
                name: "search",
                description: "Search workspace files with a regular expression (one line per match: path:line:text). include narrows files with a glob such as *.rs or src/**/*.ts; max_results caps matches (default 80).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string"},
                        "path": {"type": "string"},
                        "include": {"type": "string", "description": "Glob filter on file paths, e.g. *.rs"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 1000}
                    },
                    "required": ["pattern"]
                }),
                read_only: true,
            },
            ToolSpec {
                name: "glob",
                description: "Find files by name pattern, newest first. Bare patterns like *.rs match anywhere; patterns with / such as src/**/*.ts match relative to path. target/, node_modules/, .git/ are skipped.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string"},
                        "path": {"type": "string", "default": "."}
                    },
                    "required": ["pattern"]
                }),
                read_only: true,
            },
            ToolSpec {
                name: "shell",
                description: "Run a shell command in the workspace. Requires approval unless auto-approve is enabled. timeout_seconds caps foreground commands (default 30, max 600).",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "command": {"type": "string"},
                        "background": {"type": "boolean"},
                        "pty": {"type": "boolean"},
                        "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 600}
                    },
                    "required": ["command"]
                }),
                read_only: false,
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
                read_only: false,
            },
            ToolSpec {
                name: "git_status",
                description: "Show git status for the workspace.",
                parameters: json!({"type": "object", "properties": {}}),
                read_only: true,
            },
            ToolSpec {
                name: "git_diff",
                description: "Show git diff --stat against HEAD.",
                parameters: json!({"type": "object", "properties": {}}),
                read_only: true,
            },
            ToolSpec {
                name: "git_log",
                description: "Show recent git commits.",
                parameters: json!({"type": "object", "properties": {}}),
                read_only: true,
            },
            ToolSpec {
                name: "git_branch",
                description: "Show git branches for the workspace.",
                parameters: json!({"type": "object", "properties": {}}),
                read_only: true,
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
                read_only: false,
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
                read_only: false,
            },
            ToolSpec {
                name: "apply_patch",
                description: "Edit a workspace file by replacing old_string with new_string. old_string must match exactly once; include enough surrounding lines to make it unique. Set replace_all to change every occurrence.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "old_string": {"type": "string"},
                        "new_string": {"type": "string"},
                        "replace_all": {"type": "boolean"}
                    },
                    "required": ["path", "old_string", "new_string"]
                }),
                read_only: false,
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
                read_only: false,
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
                read_only: false,
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
                read_only: true,
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
                read_only: false,
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
                read_only: true,
            },
            ToolSpec {
                name: "update_plan",
                description: "Replace your working checklist. Send the full list every time; keep at most one step in_progress. Use it for multi-step work so the user can follow progress.",
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "steps": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "title": {"type": "string"},
                                    "status": {"type": "string", "enum": ["pending", "in_progress", "done"]}
                                },
                                "required": ["title", "status"]
                            }
                        },
                        "note": {"type": "string", "description": "Optional one-line explanation of the change"}
                    },
                    "required": ["steps"]
                }),
                read_only: true,
            },
        ]
    }

    /// True for tools that never mutate workspace, processes, or memory.
    #[must_use]
    pub fn is_read_only(name: &str) -> bool {
        Self::specs()
            .iter()
            .any(|spec| spec.name == name && spec.read_only)
    }

    pub fn execute(backend: &LocalBackend, name: &str, arguments: &Value) -> Result<String> {
        match name {
            "read_file" => {
                let text = backend.read_file(required_str(arguments, "path")?)?;
                let offset = arguments.get("offset").and_then(Value::as_u64);
                let limit = arguments.get("limit").and_then(Value::as_u64);
                Ok(slice_lines(&text, offset, limit))
            }
            "write_file" => {
                backend.write_file(
                    required_str(arguments, "path")?,
                    required_str(arguments, "contents")?,
                )?;
                Ok("wrote file".to_owned())
            }
            "list_dir" => backend.list_dir(optional_str(arguments, "path").unwrap_or(".")),
            "search" => backend.search_with(
                required_str(arguments, "pattern")?,
                optional_str(arguments, "path"),
                optional_str(arguments, "include"),
                arguments
                    .get("max_results")
                    .and_then(Value::as_u64)
                    .map_or(80, |n| n as usize),
            ),
            "glob" => backend.glob(
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
                } else if arguments
                    .get("pty")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    || std::env::var("BLORA_PTY").ok().as_deref() == Some("1")
                {
                    backend.shell_pty(command)
                } else {
                    let timeout = arguments
                        .get("timeout_seconds")
                        .and_then(Value::as_u64)
                        .map(|secs| std::time::Duration::from_secs(secs.clamp(1, 600)));
                    backend.shell_with_timeout(command, timeout)
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
                let replace_all = arguments
                    .get("replace_all")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let replaced = backend.apply_patch(
                    required_str(arguments, "path")?,
                    required_str(arguments, "old_string")?,
                    required_str(arguments, "new_string")?,
                    replace_all,
                )?;
                Ok(format!("patched file ({replaced} replacement(s))"))
            }
            other => Err(BloraError::Other(format!("unknown tool: {other}"))),
        }
    }
}

fn slice_lines(text: &str, offset: Option<u64>, limit: Option<u64>) -> String {
    if offset.is_none() && limit.is_none() {
        return text.to_owned();
    }
    let start = offset.unwrap_or(1).max(1) as usize - 1;
    let take = limit.unwrap_or(u64::MAX) as usize;
    let total = text.lines().count();
    let mut out: Vec<String> = text
        .lines()
        .enumerate()
        .skip(start)
        .take(take)
        .map(|(index, line)| format!("{:>5}\t{line}", index + 1))
        .collect();
    if start + out.len() < total {
        out.push(format!("… ({} more lines)", total - start - out.len()));
    }
    out.join("\n")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_classification() {
        assert!(ToolRegistry::is_read_only("read_file"));
        assert!(ToolRegistry::is_read_only("search"));
        assert!(ToolRegistry::is_read_only("glob"));
        assert!(ToolRegistry::is_read_only("update_plan"));
        assert!(!ToolRegistry::is_read_only("shell"));
        assert!(!ToolRegistry::is_read_only("apply_patch"));
    }

    #[test]
    fn slices_lines_with_numbers() {
        let text = "a\nb\nc\nd";
        let out = slice_lines(text, Some(2), Some(2));
        assert!(out.contains("    2\tb"));
        assert!(out.contains("    3\tc"));
        assert!(out.contains("1 more lines"));
        assert_eq!(slice_lines(text, None, None), text);
    }
}
