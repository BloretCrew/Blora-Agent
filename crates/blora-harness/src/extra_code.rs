// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use crate::{HarnessError, HarnessEvent, Outcome, Result, RunRequest};
use blora_policy::PermissionMode;
use blora_types::HarnessKind;
use serde_json::Value;
use std::collections::HashSet;
use std::ffi::OsString;

pub(crate) fn invocation(request: &RunRequest) -> Result<Vec<OsString>> {
    use HarnessKind::*;
    use PermissionMode::*;
    if let Some(id) = &request.resume_session_id {
        let valid = match request.kind {
            OpenCode => id.strip_prefix("ses_").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_alphanumeric())
            }),
            QwenCode => uuid(id),
            DeepSeekHarness => false,
            _ => false,
        };
        if !valid {
            return Err(HarnessError::Protocol(
                "resume requires an explicit CLI session ID, never last or a title".into(),
            ));
        }
    }
    let mut args: Vec<OsString> = match request.kind {
        OpenCode => {
            if !matches!(request.permission, Ask | Yolo) {
                return Err(HarnessError::UnsupportedPermission(
                    "OpenCode has no verified acceptEdits or read-only sandbox CLI mapping".into(),
                ));
            }
            let mut args: Vec<OsString> = ["run", "--format", "json", "--thinking"]
                .into_iter()
                .map(Into::into)
                .collect();
            if request.permission == Yolo {
                args.push("--auto".into());
            }
            if let Some(id) = &request.resume_session_id {
                args.extend(["--session".into(), id.into()]);
            }
            args
        }
        QwenCode => {
            let mode = match request.permission {
                Ask => "default",
                AutoEdit => "auto-edit",
                Yolo => "yolo",
                Plan => return Err(HarnessError::UnsupportedPermission(
                    "Qwen plan mode is not a verified read-only sandbox; external allow rules remain effective".into(),
                )),
            };
            // Complete messages avoid speculative partial output from retried attempts.
            let mut args: Vec<OsString> =
                ["--output-format", "stream-json", "--approval-mode", mode]
                    .into_iter()
                    .map(Into::into)
                    .collect();
            if let Some(id) = &request.resume_session_id {
                args.extend(["--resume".into(), id.into()]);
            }
            args
        }
        DeepSeekHarness => {
            if request.permission != Yolo {
                return Err(HarnessError::UnsupportedPermission(
                    "DeepSeek headless cannot override its configured permissions; only explicit bypassPermissions authorizes execution under the external profile's own policies".into(),
                ));
            }
            ["--profile", "headless"]
                .into_iter()
                .map(Into::into)
                .collect()
        }
        _ => return Err(HarnessError::Protocol("not an extra-code harness".into())),
    };
    args.push("--".into());
    args.push(request.prompt.clone().into());
    Ok(args)
}

fn uuid(id: &str) -> bool {
    let parts: Vec<_> = id.split('-').collect();
    parts.iter().map(|part| part.len()).eq([8, 4, 4, 4, 12])
        && parts
            .iter()
            .all(|part| part.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub(crate) struct CodeParser {
    kind: HarnessKind,
    outcome: Outcome,
    terminal: bool,
    text_seen: bool,
    plaintext: bool,
    seen: HashSet<String>,
    started: HashSet<String>,
    finished: HashSet<String>,
    usage: (u64, u64, u64),
    records: usize,
}

impl CodeParser {
    pub(crate) fn new(kind: HarnessKind) -> Self {
        Self {
            kind,
            outcome: Outcome::default(),
            terminal: false,
            text_seen: false,
            plaintext: false,
            seen: HashSet::new(),
            started: HashSet::new(),
            finished: HashSet::new(),
            usage: (0, 0, 0),
            records: 0,
        }
    }

    pub(crate) fn line(&mut self, v: &Value, emit: &mut impl FnMut(HarnessEvent)) -> Result<()> {
        if self.kind == HarnessKind::DeepSeekHarness {
            if let Some(text) = v.as_str() {
                if self.records != 0 || text.trim().is_empty() {
                    return Err(HarnessError::Protocol(
                        "DeepSeek plaintext requires one non-empty final stdout document".into(),
                    ));
                }
                self.records = 1;
                self.plaintext = true;
                self.terminal = true;
                emit(HarnessEvent::Warning {
                    message: "DeepSeek headless reports final stdout only: no tool/usage/session protocol; each request starts fresh and external profile permissions remain effective".into(),
                });
                self.text(text, false, emit);
                return Ok(());
            }
        }
        if self.plaintext || !v.is_object() {
            return Err(HarnessError::Protocol(
                "expected an event object, or one DeepSeek plaintext document".into(),
            ));
        }
        self.records += 1;
        if self.records > 100_000 {
            return Err(HarnessError::Protocol("too many protocol records".into()));
        }
        match self.kind {
            HarnessKind::OpenCode => self.opencode(v, emit),
            HarnessKind::QwenCode => self.qwen(v, emit),
            HarnessKind::DeepSeekHarness => self.deepseek(v, emit),
            _ => Err(HarnessError::Protocol("not an extra-code harness".into())),
        }
    }

    pub(crate) fn finish(self) -> Result<Outcome> {
        if !self.terminal || (!self.plaintext && self.outcome.session_id.is_none()) {
            return Err(HarnessError::Protocol(
                "missing successful completion or session ID; check CLI version/authentication"
                    .into(),
            ));
        }
        Ok(self.outcome)
    }

    fn session(
        &mut self,
        id: &str,
        model: Option<&str>,
        emit: &mut impl FnMut(HarnessEvent),
    ) -> Result<()> {
        if id.is_empty() {
            return Ok(());
        }
        if self
            .outcome
            .session_id
            .as_deref()
            .is_some_and(|old| old != id)
        {
            return Err(HarnessError::Protocol(
                "session ID changed during run".into(),
            ));
        }
        let changed = self.outcome.session_id.is_none()
            || model.is_some_and(|m| self.outcome.model.as_deref() != Some(m));
        self.outcome.session_id = Some(id.into());
        if let Some(model) = model {
            self.outcome.model = Some(model.into());
        }
        if changed {
            emit(HarnessEvent::Session {
                session_id: id.into(),
                model: self.outcome.model.clone(),
            });
        }
        Ok(())
    }

    fn text(&mut self, text: &str, reasoning: bool, emit: &mut impl FnMut(HarnessEvent)) {
        if text.is_empty() {
            return;
        }
        if reasoning {
            emit(HarnessEvent::Reasoning { text: text.into() });
        } else {
            self.text_seen = true;
            emit(HarnessEvent::Text { text: text.into() });
        }
    }

    fn start(
        &mut self,
        id: &str,
        name: &str,
        input: Value,
        emit: &mut impl FnMut(HarnessEvent),
    ) -> Result<()> {
        if id.is_empty() || name.is_empty() {
            return Err(HarnessError::Protocol("tool call missing ID/name".into()));
        }
        if self.started.insert(id.into()) {
            emit(HarnessEvent::ToolStarted {
                call_id: id.into(),
                name: name.into(),
                input,
            });
        }
        Ok(())
    }

    fn end(
        &mut self,
        id: &str,
        output: &Value,
        error: bool,
        emit: &mut impl FnMut(HarnessEvent),
    ) -> Result<()> {
        if id.is_empty() {
            return Err(HarnessError::Protocol("tool result missing ID".into()));
        }
        if self.finished.insert(id.into()) {
            emit(HarnessEvent::ToolFinished {
                call_id: id.into(),
                output: output
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| output.to_string()),
                is_error: error,
            });
        }
        Ok(())
    }

    fn usage(
        &mut self,
        next: (u64, u64, u64),
        cumulative: bool,
        emit: &mut impl FnMut(HarnessEvent),
    ) {
        let previous = self.usage;
        self.usage = if cumulative {
            next
        } else {
            (
                previous.0.saturating_add(next.0),
                previous.1.saturating_add(next.1),
                previous.2.saturating_add(next.2),
            )
        };
        if self.usage != previous {
            emit(HarnessEvent::Usage {
                input_tokens: self.usage.0,
                output_tokens: self.usage.1,
                cached_input_tokens: self.usage.2,
            });
        }
    }

    fn opencode(&mut self, v: &Value, emit: &mut impl FnMut(HarnessEvent)) -> Result<()> {
        let kind = string(v, "type");
        if kind == "error" {
            return Err(HarnessError::Process(v["error"].to_string()));
        }
        if !matches!(
            kind,
            "text" | "reasoning" | "tool_use" | "step_start" | "step_finish"
        ) {
            return Ok(());
        }
        self.session(string(v, "sessionID"), None, emit)?;
        let part = &v["part"];
        let id = string(part, "id");
        if id.is_empty() {
            return Err(HarnessError::Protocol("OpenCode part missing ID".into()));
        }
        if !self.seen.insert(format!("{kind}:{id}")) {
            return Ok(());
        }
        match kind {
            "text" | "reasoning" => self.text(string(part, "text"), kind == "reasoning", emit),
            "step_start" => self.terminal = false,
            "step_finish" => {
                let tokens = &part["tokens"];
                self.usage(
                    (
                        number(tokens, "input"),
                        number(tokens, "output"),
                        number(&tokens["cache"], "read"),
                    ),
                    false,
                    emit,
                );
                // Tool steps also finish; only a non-tool final step marks completion.
                self.terminal = matches!(string(part, "reason"), "stop" | "length");
            }
            "tool_use" => {
                let state = &part["state"];
                let status = string(state, "status");
                if !matches!(status, "completed" | "error") {
                    return Err(HarnessError::Protocol(
                        "OpenCode tool_use is not a completed tool".into(),
                    ));
                }
                let call = string(part, "callID");
                self.start(call, string(part, "tool"), state["input"].clone(), emit)?;
                self.end(
                    call,
                    &state[if status == "error" { "error" } else { "output" }],
                    status == "error",
                    emit,
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    fn qwen(&mut self, v: &Value, emit: &mut impl FnMut(HarnessEvent)) -> Result<()> {
        // Child-agent identifiers and answers are not the foreground session.
        let child = v.get("parent_tool_use_id").is_some_and(|id| !id.is_null());
        let kind = string(v, "type");
        if !matches!(kind, "system" | "assistant" | "user" | "result") {
            return Ok(());
        }
        if !child {
            let model = v["model"]
                .as_str()
                .or_else(|| v["message"]["model"].as_str());
            self.session(string(v, "session_id"), model, emit)?;
        }
        match kind {
            "assistant" | "user" => {
                let message = &v["message"];
                let id = message["id"]
                    .as_str()
                    .or_else(|| v["uuid"].as_str())
                    .unwrap_or("");
                if let Some(blocks) = message["content"].as_array() {
                    for (index, block) in blocks.iter().enumerate() {
                        match string(block, "type") {
                            "text" | "thinking" if kind == "assistant" && !child => {
                                if id.is_empty() {
                                    return Err(HarnessError::Protocol(
                                        "Qwen assistant missing message ID".into(),
                                    ));
                                }
                                if self.seen.insert(format!("qwen:{id}:{index}")) {
                                    let reasoning = string(block, "type") == "thinking";
                                    self.text(
                                        string(block, if reasoning { "thinking" } else { "text" }),
                                        reasoning,
                                        emit,
                                    );
                                }
                            }
                            "tool_use" => self.start(
                                string(block, "id"),
                                string(block, "name"),
                                block["input"].clone(),
                                emit,
                            )?,
                            "tool_result" => self.end(
                                string(block, "tool_use_id"),
                                &block["content"],
                                block["is_error"].as_bool() == Some(true),
                                emit,
                            )?,
                            _ => {}
                        }
                    }
                }
            }
            "result" if !child => {
                if v["is_error"].as_bool() != Some(false) || string(v, "subtype") != "success" {
                    return Err(HarnessError::Process(v["error"].to_string()));
                }
                if !self.terminal {
                    if !self.text_seen {
                        self.text(string(v, "result"), false, emit);
                    }
                    let usage = &v["usage"];
                    self.usage(
                        (
                            number(usage, "input_tokens"),
                            number(usage, "output_tokens"),
                            number(usage, "cache_read_input_tokens"),
                        ),
                        true,
                        emit,
                    );
                    self.terminal = true;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn deepseek(&mut self, v: &Value, emit: &mut impl FnMut(HarnessEvent)) -> Result<()> {
        if v["truncated"].as_bool() == Some(true) {
            emit(HarnessEvent::Warning {
                message: "DeepSeek projected event was truncated by the CLI".into(),
            });
        }
        match string(v, "type") {
            "session" => self.session(string(v, "sessionId"), None, emit)?,
            // Projection text has no identity; identical text in distinct blocks is legitimate.
            "text" | "thinking" => {
                self.text(string(v, "text"), string(v, "type") == "thinking", emit)
            }
            "tool_call" => self.start(
                string(v, "callId"),
                string(v, "tool"),
                v["input"].clone(),
                emit,
            )?,
            "tool_result" => self.end(
                string(v, "callId"),
                &v["result"],
                string(v, "status") == "error",
                emit,
            )?,
            "status" => {
                if string(v, "phase") == "turn_end" && string(&v["reason"], "kind") != "completed" {
                    return Err(HarnessError::Process(format!(
                        "DeepSeek turn ended: {}",
                        v["reason"]
                    )));
                }
                if string(v, "phase") == "step_end" && v["usage"].is_object() {
                    let key = format!("dsh-usage:{}:{}", v["turn"], v["step"]);
                    if self.seen.insert(key) {
                        let usage = &v["usage"];
                        self.usage(
                            (
                                number(usage, "inputTokens"),
                                number(usage, "outputTokens"),
                                number(usage, "cacheReadTokens"),
                            ),
                            false,
                            emit,
                        );
                    }
                }
            }
            "error" => return Err(HarnessError::Process(string(v, "message").into())),
            "final" => {
                if !self.terminal && !self.text_seen {
                    self.text(string(v, "text"), false, emit);
                }
                self.terminal = true;
            }
            _ => {}
        }
        Ok(())
    }
}

fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn number(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(kind: HarnessKind, permission: PermissionMode) -> RunRequest {
        RunRequest {
            kind,
            permission,
            cwd: "/tmp".into(),
            prompt: "--resume last; echo nope".into(),
            resume_session_id: None,
        }
    }

    #[test]
    fn arguments_preserve_literal_prompt_and_explicit_sessions() {
        let mut req = request(HarnessKind::OpenCode, PermissionMode::Ask);
        req.resume_session_id = Some("ses_abc123".into());
        assert_eq!(
            invocation(&req).unwrap(),
            [
                "run",
                "--format",
                "json",
                "--thinking",
                "--session",
                "ses_abc123",
                "--",
                &req.prompt
            ]
            .map(OsString::from)
        );
        let mut req = request(HarnessKind::QwenCode, PermissionMode::AutoEdit);
        req.resume_session_id = Some("12345678-1234-1234-1234-123456789abc".into());
        assert_eq!(
            invocation(&req).unwrap(),
            [
                "--output-format",
                "stream-json",
                "--approval-mode",
                "auto-edit",
                "--resume",
                "12345678-1234-1234-1234-123456789abc",
                "--",
                &req.prompt
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn permission_modes_fail_closed() {
        for mode in [
            PermissionMode::Ask,
            PermissionMode::AutoEdit,
            PermissionMode::Plan,
        ] {
            assert!(matches!(
                invocation(&request(HarnessKind::DeepSeekHarness, mode)),
                Err(HarnessError::UnsupportedPermission(_))
            ));
        }
        for mode in [PermissionMode::AutoEdit, PermissionMode::Plan] {
            assert!(matches!(
                invocation(&request(HarnessKind::OpenCode, mode)),
                Err(HarnessError::UnsupportedPermission(_))
            ));
        }
        assert!(invocation(&request(HarnessKind::QwenCode, PermissionMode::Plan)).is_err());
        assert!(
            invocation(&request(HarnessKind::OpenCode, PermissionMode::Yolo))
                .unwrap()
                .contains(&OsString::from("--auto"))
        );
        for mode in [PermissionMode::Ask, PermissionMode::Yolo] {
            assert!(invocation(&request(HarnessKind::QwenCode, mode)).is_ok());
        }
    }

    #[test]
    fn titles_last_and_option_ids_are_rejected() {
        for kind in [
            HarnessKind::OpenCode,
            HarnessKind::QwenCode,
            HarnessKind::DeepSeekHarness,
        ] {
            for id in ["", "last", "my-title", "--continue", "ses_", "session-last"] {
                let mut req = request(kind, PermissionMode::Ask);
                req.resume_session_id = Some(id.into());
                assert!(matches!(invocation(&req), Err(HarnessError::Protocol(_))));
            }
        }
    }

    fn parse(kind: HarnessKind, rows: &[Value]) -> (CodeParser, Vec<HarnessEvent>) {
        let mut parser = CodeParser::new(kind);
        let mut events = Vec::new();
        for row in rows {
            parser.line(row, &mut |e| events.push(e)).unwrap();
        }
        (parser, events)
    }

    #[test]
    fn opencode_parts_tools_and_usage_are_deduplicated() {
        let text = json!({"type":"text","sessionID":"ses_a","part":{"id":"p1","text":"answer"}});
        let thinking =
            json!({"type":"reasoning","sessionID":"ses_a","part":{"id":"p2","text":"think"}});
        let tool = json!({"type":"tool_use","sessionID":"ses_a","part":{"id":"p3","callID":"call1","tool":"read","state":{"status":"completed","input":{},"output":"ok"}}});
        let step = json!({"type":"step_finish","sessionID":"ses_a","part":{"id":"p4","reason":"stop","tokens":{"input":2,"output":3,"cache":{"read":1}}}});
        let (parser, events) = parse(
            HarnessKind::OpenCode,
            &[
                text.clone(),
                text,
                thinking.clone(),
                thinking,
                tool.clone(),
                tool,
                step.clone(),
                step,
                json!({"type":"future"}),
            ],
        );
        assert_eq!(events.len(), 6);
        assert_eq!(
            parser.finish().unwrap().session_id.as_deref(),
            Some("ses_a")
        );
    }

    #[test]
    fn tool_step_does_not_complete_opencode() {
        let (parser, _) = parse(
            HarnessKind::OpenCode,
            &[
                json!({"type":"step_finish","sessionID":"ses_a","part":{"id":"p","reason":"tool-calls"}}),
            ],
        );
        assert!(parser.finish().is_err());
    }

    #[test]
    fn qwen_complete_messages_result_and_child_are_distinct() {
        let assistant = json!({"type":"assistant","session_id":"root","message":{"id":"m","model":"model","content":[{"type":"thinking","thinking":"think"},{"type":"text","text":"answer"},{"type":"tool_use","id":"c","name":"read","input":{}}]}});
        let result = json!({"type":"result","session_id":"root","subtype":"success","is_error":false,"result":"answer","usage":{"input_tokens":3,"output_tokens":2,"cache_read_input_tokens":1}});
        let (parser, events) = parse(
            HarnessKind::QwenCode,
            &[
                assistant.clone(),
                assistant,
                json!({"type":"assistant","parent_tool_use_id":"c","session_id":"child","message":{"id":"sub","content":[{"type":"text","text":"hidden"}]}}),
                json!({"type":"user","session_id":"root","message":{"content":[{"type":"tool_result","tool_use_id":"c","content":"ok"}]}}),
                result.clone(),
                result,
            ],
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, HarnessEvent::Text { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, HarnessEvent::Usage { .. }))
                .count(),
            1
        );
        assert_eq!(parser.finish().unwrap().model.as_deref(), Some("model"));
    }

    #[test]
    fn single_result_object_and_errors() {
        let (parser, events) = parse(
            HarnessKind::QwenCode,
            &[
                json!({"type":"result","session_id":"s","subtype":"success","is_error":false,"result":"answer"}),
            ],
        );
        assert!(parser.finish().is_ok());
        assert!(
            events
                .iter()
                .any(|e| matches!(e,HarnessEvent::Text { text } if text == "answer"))
        );
        for kind in [
            HarnessKind::OpenCode,
            HarnessKind::QwenCode,
            HarnessKind::DeepSeekHarness,
        ] {
            let mut parser = CodeParser::new(kind);
            assert!(parser.line(&json!([]), &mut |_| {}).is_err());
            assert!(parser.finish().is_err());
        }
        assert!(
            CodeParser::new(HarnessKind::QwenCode)
                .line(
                    &json!({"type":"result","is_error":true,"error":{"message":"no"}}),
                    &mut |_| {}
                )
                .is_err()
        );
    }

    #[test]
    fn deepseek_projection_and_usage_do_not_repeat_final_answer() {
        let status = json!({"type":"status","phase":"step_end","turn":1,"step":1,"usage":{"inputTokens":3,"outputTokens":4,"cacheReadTokens":2}});
        let (parser, events) = parse(
            HarnessKind::DeepSeekHarness,
            &[
                json!({"type":"session","sessionId":"session-a"}),
                json!({"type":"thinking","text":"think"}),
                json!({"type":"text","text":"answer"}),
                json!({"type":"tool_call","callId":"c","tool":"read","input":{}}),
                json!({"type":"tool_result","callId":"c","status":"error","result":"no"}),
                status.clone(),
                status,
                json!({"type":"status","phase":"turn_end","reason":{"kind":"completed"}}),
                json!({"type":"final","text":"answer"}),
            ],
        );
        assert!(parser.finish().is_ok());
        assert_eq!(events.len(), 6);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, HarnessEvent::ToolFinished { is_error: true, .. }))
        );
        assert!(
            CodeParser::new(HarnessKind::DeepSeekHarness)
                .line(
                    &json!({"type":"status","phase":"turn_end","reason":{"kind":"aborted"}}),
                    &mut |_| {}
                )
                .is_err()
        );
    }

    #[test]
    fn deepseek_yolo_uses_installed_plaintext_flags_and_rejects_resume() {
        let mut req = request(HarnessKind::DeepSeekHarness, PermissionMode::Yolo);
        assert_eq!(
            invocation(&req).unwrap(),
            ["--profile", "headless", "--", &req.prompt].map(OsString::from)
        );
        req.resume_session_id = Some("session-12345678-1234-1234-1234-123456789abc".into());
        assert!(matches!(invocation(&req), Err(HarnessError::Protocol(_))));
        let (parser, events) = parse(
            HarnessKind::DeepSeekHarness,
            &[Value::String("first\nsecond\n".into())],
        );
        assert_eq!(parser.finish().unwrap(), Outcome::default());
        assert!(matches!(&events[0], HarnessEvent::Warning { .. }));
        assert_eq!(
            events[1],
            HarnessEvent::Text {
                text: "first\nsecond\n".into()
            }
        );
        for kind in [HarnessKind::OpenCode, HarnessKind::QwenCode] {
            assert!(
                CodeParser::new(kind)
                    .line(&Value::String("answer".into()), &mut |_| {})
                    .is_err()
            );
        }
        let mut parser = CodeParser::new(HarnessKind::DeepSeekHarness);
        assert!(parser.line(&Value::String("".into()), &mut |_| {}).is_err());
        parser
            .line(&Value::String("answer".into()), &mut |_| {})
            .unwrap();
        assert!(
            parser
                .line(&json!({"type":"final","text":"answer"}), &mut |_| {})
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn fake_deepseek_stdout_is_one_final_document_with_null_stdin() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("fake-dsh");
        std::fs::write(
            &executable,
            "#!/bin/sh\nif read -r line; then exit 2; fi\nprintf 'first\\nsecond\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let req = request(HarnessKind::DeepSeekHarness, PermissionMode::Yolo);
        let output = std::process::Command::new(&executable)
            .args(invocation(&req).unwrap())
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success());
        let (parser, events) = parse(
            HarnessKind::DeepSeekHarness,
            &[Value::String(String::from_utf8(output.stdout).unwrap())],
        );
        assert_eq!(parser.finish().unwrap().session_id, None);
        assert_eq!(
            events[1],
            HarnessEvent::Text {
                text: "first\nsecond\n".into()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn fake_command_receives_prompt_as_one_argument() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("fake-code");
        std::fs::write(&executable, "#!/bin/sh\nprintf '%s\\0' \"$@\"\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        for kind in [
            HarnessKind::OpenCode,
            HarnessKind::QwenCode,
            HarnessKind::DeepSeekHarness,
        ] {
            let req = request(
                kind,
                if kind == HarnessKind::DeepSeekHarness {
                    PermissionMode::Yolo
                } else {
                    PermissionMode::Ask
                },
            );
            let args = invocation(&req).unwrap();
            let result = std::process::Command::new(&executable)
                .args(&args)
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            assert!(result.status.success());
            let expected: Vec<u8> = args
                .iter()
                .flat_map(|arg| arg.as_encoded_bytes().iter().copied().chain([0]))
                .collect();
            assert_eq!(result.stdout, expected);
        }
    }
}
