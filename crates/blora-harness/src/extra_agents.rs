// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use crate::{HarnessError, HarnessEvent, Outcome, Result, RunRequest};
use blora_policy::PermissionMode;
use blora_types::HarnessKind;
use serde_json::Value;
use std::ffi::OsString;

pub(crate) fn invocation(request: &RunRequest) -> Result<Vec<OsString>> {
    match request.kind {
        HarnessKind::HermesAgent => {
            // Both installed finite-query transports bypass approval prompts.
            if request.permission != PermissionMode::Yolo {
                return Err(HarnessError::UnsupportedPermission(
                    "Hermes headless mode automatically approves commands and hooks; only explicit bypassPermissions is supported".into(),
                ));
            }
            // Installed oneshot stdout does not expose a session ID. Replay belongs to runtime.
            if request.resume_session_id.is_some() {
                return Err(HarnessError::Protocol(
                    "Hermes oneshot continuation is unsupported; replay Blora history in a fresh request".into(),
                ));
            }
            if !request.cwd.is_absolute() || !request.cwd.is_dir() {
                return Err(HarnessError::Process(
                    "Hermes requires an existing absolute working directory".into(),
                ));
            }
            if request.prompt.trim().is_empty() {
                return Err(HarnessError::Protocol(
                    "Hermes requires a nonempty prompt".into(),
                ));
            }
            // Equals syntax preserves leading dashes without involving a shell.
            Ok(vec![
                "--in".into(),
                request.cwd.as_os_str().to_owned(),
                "--no-restore-cwd".into(),
                "--yolo".into(),
                format!("--oneshot={}", request.prompt).into(),
            ])
        }
        HarnessKind::OpenClaw => {
            require_openclaw_permission(request)?;
            let home = openclaw_home()?;
            let path = openclaw_config_path(&home, &request.cwd)?;
            openclaw_invocation(request, &path, &home)
        }
        _ => Err(HarnessError::Protocol(
            "extra agent adapter received an unrelated harness".into(),
        )),
    }
}

pub(crate) const OPENCLAW_WARNING: &str = "OpenClaw runs locally with its existing CLI configuration and tools under explicit bypass authorization; external tools may access the network, mutate files or send messages. Blora does not sandbox these tools. Automatic channel delivery is disabled.";

fn require_openclaw_permission(request: &RunRequest) -> Result<()> {
    if request.permission != PermissionMode::Yolo {
        return Err(HarnessError::UnsupportedPermission(
            "OpenClaw has no verified headless default/acceptEdits/plan mapping; only explicit bypassPermissions is supported".into(),
        ));
    }
    Ok(())
}

fn openclaw_home() -> Result<std::path::PathBuf> {
    let os_home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let home = match std::env::var("OPENCLAW_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        Some(value) => expand_openclaw_path(
            &value,
            os_home.as_deref().unwrap_or(std::path::Path::new("")),
            std::path::Path::new(""),
        )?,
        None => os_home.ok_or_else(|| {
            HarnessError::Process(
                "OpenClaw home is unknown; set an absolute HOME or OPENCLAW_HOME".into(),
            )
        })?,
    };
    if !home.is_absolute() {
        return Err(HarnessError::Process(
            "OpenClaw requires an absolute HOME or OPENCLAW_HOME".into(),
        ));
    }
    Ok(home)
}

fn expand_openclaw_path(
    raw: &str,
    home: &std::path::Path,
    cwd: &std::path::Path,
) -> Result<std::path::PathBuf> {
    let raw = raw.trim();
    if raw.is_empty()
        || raw.contains('\0')
        || raw.contains("${")
        || (raw.starts_with('~') && raw != "~" && !raw.starts_with("~/"))
    {
        return Err(HarnessError::Process(
            "OpenClaw path is empty or cannot be resolved safely".into(),
        ));
    }
    let path = if raw == "~" {
        home.to_path_buf()
    } else if let Some(tail) = raw.strip_prefix("~/") {
        home.join(tail)
    } else {
        std::path::PathBuf::from(raw)
    };
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    if !path.is_absolute() {
        return Err(HarnessError::Process(
            "OpenClaw path requires a known absolute base directory".into(),
        ));
    }
    Ok(path)
}

fn openclaw_config_path(
    home: &std::path::Path,
    cwd: &std::path::Path,
) -> Result<std::path::PathBuf> {
    if let Ok(path) = std::env::var("OPENCLAW_CONFIG_PATH") {
        if !path.trim().is_empty() {
            return expand_openclaw_path(&path, home, cwd);
        }
    }
    let mut dirs = Vec::new();
    if let Ok(state) = std::env::var("OPENCLAW_STATE_DIR") {
        if !state.trim().is_empty() {
            dirs.push(expand_openclaw_path(&state, home, cwd)?);
        }
    }
    dirs.extend([home.join(".openclaw"), home.join(".clawdbot")]);
    for dir in dirs {
        for name in ["openclaw.json", "clawdbot.json"] {
            let path = dir.join(name);
            if path.try_exists()? {
                return Ok(path);
            }
        }
    }
    Err(HarnessError::Process("OpenClaw configuration not found; configure the selected agent workspace using OpenClaw itself, or set OPENCLAW_CONFIG_PATH".into()))
}

fn contains_unresolved_config(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("$include") || object.values().any(contains_unresolved_config)
        }
        Value::Array(array) => array.iter().any(contains_unresolved_config),
        Value::String(text) => text.contains("${"),
        _ => false,
    }
}

fn openclaw_invocation(
    request: &RunRequest,
    config_path: &std::path::Path,
    home: &std::path::Path,
) -> Result<Vec<OsString>> {
    require_openclaw_permission(request)?;
    if !request.cwd.is_absolute() || !request.cwd.is_dir() {
        return Err(HarnessError::Process(
            "OpenClaw requires an existing absolute working directory".into(),
        ));
    }
    if request.prompt.trim().is_empty() {
        return Err(HarnessError::Protocol(
            "OpenClaw requires a nonempty prompt".into(),
        ));
    }
    let id = if let Some(id) = &request.resume_session_id {
        let parsed = uuid::Uuid::parse_str(id).map_err(|_| {
            HarnessError::Protocol(
                "OpenClaw continuation requires an explicitly bound UUID, never latest or a title"
                    .into(),
            )
        })?;
        if parsed.hyphenated().to_string() != *id || parsed.is_nil() {
            return Err(HarnessError::Protocol(
                "OpenClaw continuation requires a non-nil canonical UUID".into(),
            ));
        }
        id.clone()
    } else {
        uuid::Uuid::now_v7().to_string()
    };
    // Configuration is only read; unsupported JSON5/includes fail before spawning.
    let file = std::fs::File::open(config_path)?;
    if file.metadata()?.len() > 4 * 1024 * 1024 {
        return Err(HarnessError::Process(
            "OpenClaw configuration exceeds 4 MiB".into(),
        ));
    }
    let config: Value = serde_json::from_reader(file).map_err(|_| HarnessError::Process("OpenClaw configuration must be strict JSON for workspace verification; JSON5 is not supported by this adapter".into()))?;
    if !config.is_object() || contains_unresolved_config(&config) {
        return Err(HarnessError::Process("OpenClaw configuration includes or environment substitutions cannot be verified by this adapter".into()));
    }
    let agents = config
        .get("agents")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            HarnessError::Process(
                "OpenClaw requires an explicitly configured agent workspace".into(),
            )
        })?;
    let entries = match agents.get("list") {
        None => &[][..],
        Some(Value::Array(entries)) if entries.iter().all(Value::is_object) => entries.as_slice(),
        _ => return Err(HarnessError::Process("Invalid OpenClaw agent list".into())),
    };
    if entries.iter().any(|entry| {
        entry
            .get("default")
            .is_some_and(|value| !value.is_boolean())
    }) {
        return Err(HarnessError::Process(
            "Invalid OpenClaw default agent marker".into(),
        ));
    }
    let selected = entries
        .iter()
        .find(|entry| entry["default"].as_bool() == Some(true))
        .or_else(|| entries.first());
    let agent_id = match selected {
        Some(entry) => entry
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                HarnessError::Process(
                    "OpenClaw selected agent requires an explicit valid ID".into(),
                )
            })?
            .trim()
            .to_ascii_lowercase(),
        None => "main".into(),
    };
    if agent_id.is_empty()
        || agent_id.len() > 64
        || !agent_id.as_bytes()[0].is_ascii_alphanumeric()
        || !agent_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(HarnessError::Process(
            "OpenClaw selected agent ID cannot be verified safely".into(),
        ));
    }
    // Duplicate IDs can change which entry supplies the effective workspace.
    if entries
        .iter()
        .filter(|entry| {
            entry["id"]
                .as_str()
                .is_some_and(|id| id.trim().eq_ignore_ascii_case(&agent_id))
        })
        .count()
        > 1
    {
        return Err(HarnessError::Process(
            "OpenClaw has duplicate selected agent IDs".into(),
        ));
    }
    let workspace = selected.and_then(|entry| entry.get("workspace")).and_then(Value::as_str).filter(|text| !text.trim().is_empty()).or_else(|| agents["defaults"]["workspace"].as_str().filter(|text| !text.trim().is_empty())).ok_or_else(|| HarnessError::Process("OpenClaw selected agent has no explicit workspace; set it using OpenClaw's own configuration".into()))?;
    let workspace = expand_openclaw_path(workspace, home, &request.cwd)?;
    if std::fs::canonicalize(&workspace)? != std::fs::canonicalize(&request.cwd)? {
        return Err(HarnessError::Process("OpenClaw selected agent workspace differs from the Blora project; set the matching workspace using OpenClaw's own configuration".into()));
    }
    // A namespaced explicit key avoids routing a UUID to main or another agent store.
    Ok(vec![
        "agent".into(),
        "--local".into(),
        "--json".into(),
        "--agent".into(),
        agent_id.clone().into(),
        "--session-id".into(),
        id.clone().into(),
        "--session-key".into(),
        format!("agent:{agent_id}:blora:{id}").into(),
        format!("--message={}", request.prompt).into(),
    ])
}

pub(crate) struct AgentsParser {
    kind: HarnessKind,
    outcome: Outcome,
    terminal: bool,
}

impl AgentsParser {
    pub(crate) fn new(kind: HarnessKind) -> Self {
        Self {
            kind,
            outcome: Outcome::default(),
            terminal: false,
        }
    }

    // Hermes receives one complete UTF-8 stdout string at successful EOF, not a JSON record.
    pub(crate) fn line(
        &mut self,
        value: &Value,
        emit: &mut impl FnMut(HarnessEvent),
    ) -> Result<()> {
        if self.terminal {
            return Err(HarnessError::Protocol(
                "record after final agent response".into(),
            ));
        }
        match self.kind {
            HarnessKind::HermesAgent => {
                let text = value
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| {
                        HarnessError::Protocol(
                            "Hermes oneshot requires a nonempty plaintext response".into(),
                        )
                    })?;
                emit(HarnessEvent::Warning {
                    message: "Hermes oneshot returns final text only; tool events, reasoning, usage and external session continuation are unavailable".into(),
                });
                emit(HarnessEvent::Text { text: text.into() });
            }
            HarnessKind::OpenClaw => self.openclaw(value, emit)?,
            _ => {
                return Err(HarnessError::Protocol(
                    "unsupported extra agent protocol".into(),
                ));
            }
        }
        self.terminal = true;
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<Outcome> {
        if !self.terminal {
            return Err(HarnessError::Protocol(
                "missing final agent response".into(),
            ));
        }
        Ok(self.outcome)
    }

    fn openclaw(&mut self, value: &Value, emit: &mut impl FnMut(HarnessEvent)) -> Result<()> {
        // Embedded --local --json returns a single pretty-printed payloads/meta object.
        let payloads = value
            .get("payloads")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                HarnessError::Protocol("OpenClaw local result is missing payloads".into())
            })?;
        let meta = value
            .get("meta")
            .filter(|meta| meta.is_object())
            .ok_or_else(|| {
                HarnessError::Protocol("OpenClaw local result is missing meta".into())
            })?;
        if meta.get("aborted").and_then(Value::as_bool) == Some(true) {
            return Err(HarnessError::Cancelled);
        }
        if meta.get("error").is_some_and(|error| !error.is_null())
            || payloads
                .iter()
                .any(|payload| payload.get("isError").and_then(Value::as_bool) == Some(true))
        {
            return Err(HarnessError::Process(
                "OpenClaw reported an unsuccessful local result".into(),
            ));
        }
        if !payloads.iter().all(Value::is_object)
            || payloads
                .iter()
                .any(|payload| payload.get("text").is_some_and(|text| !text.is_string()))
        {
            return Err(HarnessError::Protocol("invalid OpenClaw payload".into()));
        }
        let agent = &meta["agentMeta"];
        if let Some(id) = agent
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            self.outcome.session_id = Some(id.into());
        }
        self.outcome.model = agent
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(id) = &self.outcome.session_id {
            emit(HarnessEvent::Session {
                session_id: id.clone(),
                model: self.outcome.model.clone(),
            });
        }
        emit(HarnessEvent::Warning {
            message: OPENCLAW_WARNING.into(),
        });
        emit(HarnessEvent::Warning {
            message:
                "OpenClaw local JSON is a final response snapshot, not a tool or reasoning stream"
                    .into(),
        });
        let mut text_seen = false;
        for payload in payloads {
            if let Some(text) = payload
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                if text_seen {
                    emit(HarnessEvent::Text {
                        text: "\n\n".into(),
                    });
                }
                emit(HarnessEvent::Text { text: text.into() });
                text_seen = true;
            }
            if payload.get("mediaUrl").is_some_and(|url| !url.is_null())
                || payload
                    .get("mediaUrls")
                    .and_then(Value::as_array)
                    .is_some_and(|urls| !urls.is_empty())
            {
                emit(HarnessEvent::Warning {
                    message: "OpenClaw returned media that this text adapter cannot render".into(),
                });
            }
        }
        if let Some(usage) = agent.get("usage").filter(|usage| usage.is_object()) {
            emit(HarnessEvent::Usage {
                input_tokens: usage["input"].as_u64().unwrap_or(0),
                output_tokens: usage["output"].as_u64().unwrap_or(0),
                cached_input_tokens: usage["cacheRead"].as_u64().unwrap_or(0),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(kind: HarnessKind, permission: PermissionMode) -> RunRequest {
        RunRequest {
            kind,
            permission,
            cwd: std::env::current_dir().unwrap(),
            prompt: "--flag; $(touch injected)\nhello".into(),
            resume_session_id: None,
        }
    }

    #[test]
    fn hermes_only_explicit_bypass_and_no_external_resume() {
        for permission in [
            PermissionMode::Ask,
            PermissionMode::AutoEdit,
            PermissionMode::Plan,
        ] {
            assert!(matches!(
                invocation(&request(HarnessKind::HermesAgent, permission)),
                Err(HarnessError::UnsupportedPermission(_))
            ));
        }
        let mut req = request(HarnessKind::HermesAgent, PermissionMode::Yolo);
        let args = invocation(&req).unwrap();
        assert_eq!(
            args.last().unwrap(),
            &OsString::from(format!("--oneshot={}", req.prompt))
        );
        assert!(args.contains(&OsString::from("--no-restore-cwd")));
        assert!(!args.contains(&OsString::from("--format")));
        req.resume_session_id = Some("latest".into());
        assert!(invocation(&req).is_err());
        req.resume_session_id = None;
        req.cwd = ".".into();
        assert!(invocation(&req).is_err());
    }

    #[test]
    fn openclaw_requires_explicit_bypass() {
        for permission in [
            PermissionMode::Ask,
            PermissionMode::AutoEdit,
            PermissionMode::Plan,
        ] {
            assert!(matches!(
                invocation(&request(HarnessKind::OpenClaw, permission)),
                Err(HarnessError::UnsupportedPermission(_))
            ));
        }
    }

    #[test]
    fn openclaw_verified_workspace_generates_isolated_uuid_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("openclaw.json");
        let config = json!({"agents":{"defaults":{"workspace":dir.path()},"list":[{"id":"other","workspace":"/wrong"},{"id":"Project","default":true,"workspace":dir.path()}]}});
        let bytes = serde_json::to_vec(&config).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let mut req = request(HarnessKind::OpenClaw, PermissionMode::Yolo);
        req.cwd = dir.path().to_owned();
        let args = openclaw_invocation(&req, &path, dir.path()).unwrap();
        let index = args.iter().position(|arg| arg == "--session-id").unwrap();
        let id = args[index + 1].to_str().unwrap();
        assert!(uuid::Uuid::parse_str(id).is_ok());
        assert!(args.contains(&OsString::from(format!("agent:project:blora:{id}"))));
        assert_eq!(
            args.last().unwrap(),
            &OsString::from(format!("--message={}", req.prompt))
        );
        assert!(args.contains(&OsString::from("--local")));
        assert!(args.contains(&OsString::from("--json")));
        assert!(!args.iter().any(|arg| arg == "--deliver"
            || arg == "--to"
            || arg.to_string_lossy().starts_with("--reply-")));
        let next = openclaw_invocation(&req, &path, dir.path()).unwrap();
        assert_ne!(args[index + 1], next[index + 1]);
        req.resume_session_id = Some(id.into());
        assert_eq!(openclaw_invocation(&req, &path, dir.path()).unwrap(), args);
        for invalid in [
            "latest",
            "main",
            "title",
            "--last",
            "00000000-0000-0000-0000-000000000000",
        ] {
            req.resume_session_id = Some(invalid.into());
            assert!(openclaw_invocation(&req, &path, dir.path()).is_err());
        }
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn openclaw_workspace_mismatch_unknown_and_unresolved_config_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let path = dir.path().join("openclaw.json");
        let mut req = request(HarnessKind::OpenClaw, PermissionMode::Yolo);
        req.cwd = dir.path().to_owned();
        for config in [
            json!({}),
            json!({"agents":{"defaults":{"workspace":other.path()}}}),
            json!({"agents":{"defaults":{"workspace":dir.path()},"list":[{"id":"main","workspace":other.path()}]}}),
            json!({"$include":"more.json","agents":{"defaults":{"workspace":dir.path()}}}),
            json!({"agents":{"defaults":{"workspace":"${PROJECT}"}}}),
            json!({"agents":{"defaults":{"workspace":dir.path()},"list":[{"id":"main"},{"id":"main"}]}}),
        ] {
            std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
            assert!(openclaw_invocation(&req, &path, dir.path()).is_err());
        }
        std::fs::write(&path, "{ // JSON5\n agents: {} }").unwrap();
        assert!(openclaw_invocation(&req, &path, dir.path()).is_err());
        assert!(openclaw_invocation(&req, &dir.path().join("missing.json"), dir.path()).is_err());
        req.cwd = ".".into();
        assert!(openclaw_invocation(&req, &path, dir.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn openclaw_workspace_canonicalization_accepts_symlink_alias() {
        let dir = tempfile::tempdir().unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(dir.path(), &alias).unwrap();
        let path = dir.path().join("openclaw.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({"agents":{"defaults":{"workspace":alias}}})).unwrap(),
        )
        .unwrap();
        let mut req = request(HarnessKind::OpenClaw, PermissionMode::Yolo);
        req.cwd = dir.path().to_owned();
        assert!(openclaw_invocation(&req, &path, dir.path()).is_ok());
    }

    #[test]
    fn hermes_plaintext_is_not_json_and_has_no_binding() {
        let mut parser = AgentsParser::new(HarnessKind::HermesAgent);
        let mut events = Vec::new();
        let text = "{\"type\":\"error\"}\nhello\n";
        parser
            .line(&json!(text), &mut |event| events.push(event))
            .unwrap();
        assert_eq!(parser.finish().unwrap(), Outcome::default());
        assert_eq!(
            events.last(),
            Some(&HarnessEvent::Text { text: text.into() })
        );
    }

    #[test]
    fn openclaw_local_pretty_json_snapshot() {
        let value = json!({"payloads":[{"text":"hello"},{"text":"world"}],"meta":{"agentMeta":{"sessionId":"bound-id","model":"model","usage":{"input":9,"output":4,"cacheRead":2}}}});
        let pretty = serde_json::to_string_pretty(&value).unwrap();
        assert!(pretty.contains('\n'));
        let mut parser = AgentsParser::new(HarnessKind::OpenClaw);
        let mut events = Vec::new();
        parser
            .line(&serde_json::from_str(&pretty).unwrap(), &mut |event| {
                events.push(event)
            })
            .unwrap();
        assert!(parser.line(&value, &mut |_| {}).is_err());
        let outcome = parser.finish().unwrap();
        assert_eq!(outcome.session_id.as_deref(), Some("bound-id"));
        assert!(events.contains(&HarnessEvent::Usage {
            input_tokens: 9,
            output_tokens: 4,
            cached_input_tokens: 2
        }));
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    HarnessEvent::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>(),
            "hello\n\nworld"
        );
    }

    #[test]
    fn malformed_missing_and_failed_results_are_rejected() {
        assert!(
            AgentsParser::new(HarnessKind::HermesAgent)
                .finish()
                .is_err()
        );
        for value in [json!(""), json!({"type":"result"})] {
            assert!(
                AgentsParser::new(HarnessKind::HermesAgent)
                    .line(&value, &mut |_| {})
                    .is_err()
            );
        }
        for value in [
            json!({}),
            json!({"payloads":[],"meta":{"error":{"message":"secret"}}}),
            json!({"payloads":[{"text":7}],"meta":{}}),
            json!({"status":"ok","result":{"payloads":[],"meta":{}}}),
            json!({"payloads":[{"text":"secret","isError":true}],"meta":{}}),
        ] {
            let mut emitted = Vec::new();
            assert!(
                AgentsParser::new(HarnessKind::OpenClaw)
                    .line(&value, &mut |event| emitted.push(event))
                    .is_err()
            );
            assert!(emitted.is_empty());
        }
        assert!(matches!(
            AgentsParser::new(HarnessKind::OpenClaw)
                .line(&json!({"payloads":[],"meta":{"aborted":true}}), &mut |_| {}),
            Err(HarnessError::Cancelled)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn fake_stdout_preserves_plaintext_and_multiline_json() {
        for (kind, fixture) in [
            (HarnessKind::HermesAgent, "answer\nsecond line\n"),
            (
                HarnessKind::OpenClaw,
                "{\n  \"payloads\": [{\"text\": \"answer\"}],\n  \"meta\": {}\n}\n",
            ),
        ] {
            let output = std::process::Command::new("/bin/sh")
                .args(["-c", "printf '%s' \"$1\"", "fake-agent", fixture])
                .output()
                .unwrap();
            assert!(output.status.success());
            let value = if kind == HarnessKind::HermesAgent {
                Value::String(String::from_utf8(output.stdout).unwrap())
            } else {
                serde_json::from_slice(&output.stdout).unwrap()
            };
            let mut parser = AgentsParser::new(kind);
            let mut events = Vec::new();
            parser
                .line(&value, &mut |event| events.push(event))
                .unwrap();
            parser.finish().unwrap();
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, HarnessEvent::Text { .. }))
            );
        }
    }
}
