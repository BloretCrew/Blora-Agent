// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! External authentication and configuration remain owned by each CLI.
//! Native execution is routed by runtime, never spawned by this crate.

mod process;
mod protocol;

use blora_policy::PermissionMode;
use blora_types::{CancelToken, HarnessKind};
use serde_json::Value;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct RunRequest {
    pub kind: HarnessKind,
    pub cwd: PathBuf,
    pub prompt: String,
    pub resume_session_id: Option<String>,
    pub permission: PermissionMode,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HarnessEvent {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    ToolStarted {
        call_id: String,
        name: String,
        input: Value,
    },
    ToolFinished {
        call_id: String,
        output: String,
        is_error: bool,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
    },
    Session {
        session_id: String,
        model: Option<String>,
    },
    Warning {
        message: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub session_id: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("external run cancelled")]
    Cancelled,
    #[error("{0} is not installed on PATH")]
    NotInstalled(HarnessKind),
    #[error("unsupported permission mapping: {0}")]
    UnsupportedPermission(String),
    #[error("incompatible CLI protocol (check CLI version): {0}")]
    Protocol(String),
    #[error("external CLI failed: {0}")]
    Process(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, HarnessError>;

/// Blora's empty path is a native-availability sentinel, not an executable.
pub fn detect(kind: HarnessKind) -> Option<PathBuf> {
    detect_in(kind, std::env::var_os("PATH").as_deref())
}

fn detect_in(kind: HarnessKind, path: Option<&OsStr>) -> Option<PathBuf> {
    let name = match kind {
        HarnessKind::Blora => return Some(PathBuf::new()),
        HarnessKind::ClaudeCode => "claude",
        HarnessKind::Codex => "codex",
        HarnessKind::GrokBuild => "grok",
    };
    for dir in std::env::split_paths(path?) {
        #[cfg(not(windows))]
        let candidates = vec![dir.join(name)];
        #[cfg(windows)]
        let candidates: Vec<_> = std::env::var("PATHEXT")
            .unwrap_or(".EXE;.CMD;.BAT;.COM".into())
            .split(';')
            .map(|ext| dir.join(format!("{name}{ext}")))
            .collect();
        for candidate in candidates {
            if executable(&candidate) {
                return std::path::absolute(candidate).ok();
            }
        }
    }
    None
}

#[cfg(unix)]
fn detect_in_executable(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|candidate| executable(candidate))
        .and_then(|candidate| std::path::absolute(candidate).ok())
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Synchronous streaming runner. Callbacks run on the caller's thread.
pub fn run(
    request: &RunRequest,
    cancel: &CancelToken,
    mut callback: impl FnMut(HarnessEvent),
) -> Result<Outcome> {
    let args = invocation(request)?;
    if cancel.is_cancelled() {
        return Err(HarnessError::Cancelled);
    }
    let path = detect(request.kind).ok_or(HarnessError::NotInstalled(request.kind))?;
    if request.kind == HarnessKind::ClaudeCode && request.permission == PermissionMode::Ask {
        callback(HarnessEvent::Warning {
            message: "Claude noninteractive default confirmation uses dontAsk: tools requiring approval are denied, not interactively confirmed; external pre-approved rules still apply".into(),
        });
    }
    process::execute(&path, request, &args, cancel, callback)
}

fn invocation(request: &RunRequest) -> Result<Vec<OsString>> {
    use HarnessKind::*;
    use PermissionMode::*;
    if request.kind == Blora {
        return Err(HarnessError::Process(
            "Blora must use the native runtime".into(),
        ));
    }
    if let Some(id) = &request.resume_session_id {
        if id.is_empty()
            || id.starts_with('-')
            || id.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '-'))
        {
            return Err(HarnessError::Protocol(
                "resume requires an explicit alphanumeric session ID".into(),
            ));
        }
        if matches!(request.kind, ClaudeCode | GrokBuild) && uuid_shape(id).is_none() {
            return Err(HarnessError::Protocol(
                "Claude/Grok resume requires a UUID, never a title".into(),
            ));
        }
    }
    let mut args: Vec<OsString> = match request.kind {
        ClaudeCode => {
            let mode = match request.permission {
                Ask => "dontAsk",
                AutoEdit => "acceptEdits",
                Yolo => "bypassPermissions",
                Plan => {
                    return Err(HarnessError::UnsupportedPermission(
                        "Claude plan does not enforce a read-only sandbox".into(),
                    ));
                }
            };
            [
                "--print",
                "--verbose",
                "--output-format",
                "stream-json",
                "--include-partial-messages",
                "--permission-mode",
                mode,
            ]
            .into_iter()
            .map(Into::into)
            .collect()
        }
        Codex => {
            if request.permission == AutoEdit {
                return Err(HarnessError::UnsupportedPermission("Codex workspace-write also permits mutating shell commands; acceptEdits is unsupported".into()));
            }
            let mut a: Vec<OsString> = ["exec", "-c", "approval_policy=\"never\""]
                .into_iter()
                .map(Into::into)
                .collect();
            if request.permission == Yolo {
                a.push("--dangerously-bypass-approvals-and-sandbox".into());
            } else {
                a.extend(["-c".into(), "sandbox_mode=\"read-only\"".into()]);
            }
            if request.resume_session_id.is_some() {
                a.push("resume".into());
            }
            a.extend(["--json".into(), "--skip-git-repo-check".into()]);
            a
        }
        GrokBuild => {
            let mode = match request.permission {
                Ask => "default",
                AutoEdit => "acceptEdits",
                Yolo => "bypassPermissions",
                Plan => {
                    return Err(HarnessError::UnsupportedPermission(
                        "Grok plan has no verified read-only sandbox mapping".into(),
                    ));
                }
            };
            [
                "--output-format",
                "streaming-json",
                "--permission-mode",
                mode,
            ]
            .into_iter()
            .map(Into::into)
            .collect()
        }
        Blora => unreachable!(),
    };
    if let Some(id) = &request.resume_session_id {
        if request.kind != Codex {
            args.push("--resume".into());
        }
        args.push(id.into());
    }
    match request.kind {
        GrokBuild => {
            args.push(format!("--single={}", request.prompt).into());
        }
        _ => {
            args.push("--".into());
            args.push(request.prompt.clone().into());
        }
    }
    Ok(args)
}

fn uuid_shape(id: &str) -> Option<()> {
    let chunks: Vec<_> = id.split('-').collect();
    (chunks.iter().map(|s| s.len()).eq([8, 4, 4, 4, 12])
        && chunks
            .iter()
            .all(|s| s.chars().all(|c| c.is_ascii_hexdigit())))
    .then_some(())
}

#[cfg(test)]
mod tests;
