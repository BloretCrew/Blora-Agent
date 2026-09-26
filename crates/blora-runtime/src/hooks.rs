// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! External hooks with the de-facto standard contract: JSON on stdin, JSON on
//! stdout, exit code as the coarse decision.
//!
//! * exit 0 with no output, or `{"decision":"allow"}` — continue.
//! * exit 2, or `{"decision":"block","reason":...}` — refuse the action.
//! * `{"decision":"ask"}` — force an interactive approval for the action.
//! * `{"additional_context": "..."}` — text appended to the tool result or prompt.
//!
//! Hooks live in `BLORA_HOOKS_DIR/<event>`; a missing hook is a no-op. A hook that
//! errors or times out degrades to `allow` and the degradation is recorded so the
//! audit trail can tell "allowed" from "not checked".

use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

pub const SESSION_START: &str = "session-start";
pub const USER_PROMPT_SUBMIT: &str = "user-prompt-submit";
pub const TOOL_BEFORE: &str = "tool-before";
pub const TOOL_AFTER: &str = "tool-after";
pub const PRE_COMPACT: &str = "pre-compact";
pub const STOP: &str = "stop";

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookDecision {
    Allow,
    Block,
    Ask,
}

impl HookDecision {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Block => "block",
            Self::Ask => "ask",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HookOutcome {
    pub decision: HookDecision,
    pub reason: Option<String>,
    pub additional_context: Option<String>,
    /// Hook errored or timed out; the decision is a fail-open default.
    pub degraded: bool,
    /// True when no hook is installed for this event.
    pub absent: bool,
}

impl HookOutcome {
    fn absent() -> Self {
        Self {
            decision: HookDecision::Allow,
            reason: None,
            additional_context: None,
            degraded: false,
            absent: true,
        }
    }

    fn degraded(reason: String) -> Self {
        Self {
            decision: HookDecision::Allow,
            reason: Some(reason),
            additional_context: None,
            degraded: true,
            absent: false,
        }
    }
}

/// Run the hook for `event` with a JSON payload. Never panics; never blocks longer
/// than the timeout.
pub fn run(event: &str, payload: &Value) -> HookOutcome {
    let dir = std::env::var_os("BLORA_HOOKS_DIR").map(std::path::PathBuf::from);
    run_in(dir.as_deref(), event, payload)
}

/// Same as [`run`] with an explicit hooks directory.
pub fn run_in(dir: Option<&std::path::Path>, event: &str, payload: &Value) -> HookOutcome {
    let Some(dir) = dir else {
        return HookOutcome::absent();
    };
    let path = dir.join(event);
    if !path.is_file() {
        return HookOutcome::absent();
    }
    let mut child = match hook_command(&path)
        .env("BLORA_HOOK", event)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            tracing::warn!("hook {event} failed to start: {err}");
            return HookOutcome::degraded(format!("failed to start: {err}"));
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let body = json!({"hook": event, "payload": payload}).to_string();
        let _ = stdin.write_all(body.as_bytes());
    }
    let mut stdout = child.stdout.take();
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut output = String::new();
        if let Some(reader) = stdout.as_mut() {
            let _ = reader.read_to_string(&mut output);
        }
        let status = child.wait();
        let _ = tx.send((status, output));
    });
    match rx.recv_timeout(TIMEOUT) {
        Ok((Ok(status), output)) => interpret(event, status.code(), &output),
        Ok((Err(err), _)) => {
            tracing::warn!("hook {event} wait failed: {err}");
            HookOutcome::degraded(format!("wait failed: {err}"))
        }
        Err(_) => {
            let _ = blora_exec::terminate_pid(pid);
            tracing::warn!("hook {event} timed out");
            HookOutcome::degraded("timed out".to_owned())
        }
    }
}

/// Fire-and-forget variant for informational events.
pub fn fire(event: &str, payload: &str) {
    let _ = run(event, &json!({"text": payload}));
}

fn hook_command(path: &Path) -> std::process::Command {
    #[cfg(windows)]
    {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if extension == "ps1" {
            let mut cmd = blora_exec::command("powershell.exe");
            cmd.args(["-NoProfile", "-NonInteractive", "-File"])
                .arg(path);
            return cmd;
        }
        if matches!(extension.as_str(), "exe" | "cmd" | "bat") {
            return blora_exec::command(path);
        }
        // Shebang scripts run when Git Bash or another POSIX shell is the host shell.
        if blora_exec::host_shell() == blora_exec::HostShell::Posix {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            let script = format!("exec {}", blora_exec::quote(&path.display().to_string()));
            return blora_exec::shell_command(parent, &script);
        }
        return blora_exec::command(path);
    }
    #[cfg(not(windows))]
    {
        blora_exec::command(path)
    }
}

fn interpret(event: &str, code: Option<i32>, output: &str) -> HookOutcome {
    let parsed: Option<Value> = serde_json::from_str(output.trim()).ok();
    let reason = parsed
        .as_ref()
        .and_then(|v| v.get("reason"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let additional_context = parsed
        .as_ref()
        .and_then(|v| {
            v.get("additional_context")
                .or_else(|| v.get("additionalContext"))
        })
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(ToOwned::to_owned);
    let explicit = parsed
        .as_ref()
        .and_then(|v| v.get("decision"))
        .and_then(Value::as_str)
        .map(|d| d.to_ascii_lowercase());
    let decision = match (explicit.as_deref(), code) {
        (Some("block" | "deny"), _) => HookDecision::Block,
        (Some("ask"), _) => HookDecision::Ask,
        (Some("allow" | "approve"), _) => HookDecision::Allow,
        (_, Some(2)) => HookDecision::Block,
        (_, Some(0)) => HookDecision::Allow,
        (_, other) => {
            tracing::warn!("hook {event} exited {}", other.unwrap_or(-1));
            return HookOutcome::degraded(format!("exit {}", other.unwrap_or(-1)));
        }
    };
    HookOutcome {
        decision,
        reason: reason.or_else(|| {
            let trimmed = output.trim();
            if parsed.is_none() && !trimmed.is_empty() {
                Some(trimmed.chars().take(400).collect())
            } else {
                None
            }
        }),
        additional_context,
        degraded: false,
        absent: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interprets_exit_codes_and_json() {
        assert_eq!(interpret("t", Some(0), "").decision, HookDecision::Allow);
        assert_eq!(interpret("t", Some(2), "no").decision, HookDecision::Block);
        let asked = interpret("t", Some(0), r#"{"decision":"ask","reason":"risky"}"#);
        assert_eq!(asked.decision, HookDecision::Ask);
        assert_eq!(asked.reason.as_deref(), Some("risky"));
        let ctx = interpret("t", Some(0), r#"{"additional_context":"note"}"#);
        assert_eq!(ctx.additional_context.as_deref(), Some("note"));
        let bad = interpret("t", Some(1), "");
        assert!(bad.degraded);
        assert_eq!(bad.decision, HookDecision::Allow);
    }

    #[cfg(unix)]
    #[test]
    fn runs_executable_hook_with_stdin_payload() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let hook = dir.path().join("tool-before");
        std::fs::write(
            &hook,
            "#!/bin/sh\ncat >/dev/null\necho '{\"decision\":\"block\",\"reason\":\"nope\"}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let outcome = run_in(Some(dir.path()), TOOL_BEFORE, &json!({"tool": "shell"}));
        assert_eq!(outcome.decision, HookDecision::Block);
        assert_eq!(outcome.reason.as_deref(), Some("nope"));
        assert!(!outcome.degraded);
        assert!(run_in(Some(dir.path()), STOP, &json!({})).absent);
    }
}
