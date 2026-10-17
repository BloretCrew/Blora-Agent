// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use super::*;
use crate::protocol::Parser;
use serde_json::json;

fn request(kind: HarnessKind, permission: PermissionMode) -> RunRequest {
    RunRequest {
        kind,
        permission,
        cwd: PathBuf::from("."),
        prompt: "--evil; $(touch injected)\nhello".into(),
        resume_session_id: None,
    }
}

fn parse(kind: HarnessKind, lines: &[Value]) -> (Outcome, Vec<HarnessEvent>) {
    let mut parser = Parser::new(kind);
    let mut events = Vec::new();
    for line in lines {
        parser
            .line(line.to_string().as_bytes(), &mut |e| events.push(e))
            .unwrap();
    }
    (parser.finish().unwrap(), events)
}

#[test]
fn arguments_are_structured_and_only_explicit_bypass() {
    for kind in [
        HarnessKind::ClaudeCode,
        HarnessKind::Codex,
        HarnessKind::GrokBuild,
    ] {
        for permission in [PermissionMode::Ask, PermissionMode::Yolo] {
            let req = request(kind, permission);
            let args = invocation(&req).unwrap();
            let prompt_arg = if kind == HarnessKind::GrokBuild {
                format!("--single={}", req.prompt)
            } else {
                req.prompt.clone()
            };
            assert_eq!(args.last().unwrap(), &OsString::from(prompt_arg));
            let dangerous = args.iter().any(|a| a.to_string_lossy().contains("bypass"));
            assert_eq!(dangerous, permission == PermissionMode::Yolo);
            assert!(
                !args
                    .iter()
                    .any(|a| a == "--continue" || a == "--last" || a == "--always-approve")
            );
        }
    }
}

#[test]
fn permissions_fail_closed() {
    for kind in [HarnessKind::ClaudeCode, HarnessKind::GrokBuild] {
        assert!(matches!(
            invocation(&request(kind, PermissionMode::Plan)),
            Err(HarnessError::UnsupportedPermission(_))
        ));
        assert!(invocation(&request(kind, PermissionMode::AutoEdit)).is_ok());
    }
    assert!(matches!(
        invocation(&request(HarnessKind::Codex, PermissionMode::AutoEdit)),
        Err(HarnessError::UnsupportedPermission(_))
    ));
    let args = invocation(&request(HarnessKind::Codex, PermissionMode::Plan)).unwrap();
    assert!(args.contains(&"sandbox_mode=\"read-only\"".into()));
    assert!(args.contains(&"approval_policy=\"never\"".into()));
    assert!(invocation(&request(HarnessKind::Blora, PermissionMode::Ask)).is_err());
}

#[test]
fn claude_confirmation_denies_unapproved_tools_without_prompting() {
    let args = invocation(&request(HarnessKind::ClaudeCode, PermissionMode::Ask)).unwrap();
    let index = args
        .iter()
        .position(|arg| arg == "--permission-mode")
        .unwrap();
    assert_eq!(args[index + 1], "dontAsk");
    assert!(!args.contains(&"manual".into()));
    assert!(!args.contains(&"default".into()));
}

#[test]
fn resume_is_explicit_and_never_a_title() {
    let id = "01a10c01-7571-71c2-9f0f-62886fc7be5b";
    for kind in [
        HarnessKind::ClaudeCode,
        HarnessKind::Codex,
        HarnessKind::GrokBuild,
    ] {
        let mut req = request(kind, PermissionMode::Ask);
        req.resume_session_id = Some(id.into());
        let args = invocation(&req).unwrap();
        let index = args.iter().position(|a| a == id).unwrap();
        assert_eq!(
            args[index - 1],
            if kind == HarnessKind::Codex {
                "--skip-git-repo-check"
            } else {
                "--resume"
            }
        );
        if kind == HarnessKind::Codex {
            assert!(args.contains(&"resume".into()));
        }
        for invalid in ["", "--last", "a b", "a/b"] {
            req.resume_session_id = Some(invalid.into());
            assert!(invocation(&req).is_err());
        }
    }
    let mut req = request(HarnessKind::GrokBuild, PermissionMode::Ask);
    req.resume_session_id = Some("title".into());
    assert!(invocation(&req).is_err());
}

#[test]
fn claude_stream_snapshot_final_and_child_are_not_duplicated() {
    let (out, events) = parse(
        HarnessKind::ClaudeCode,
        &[
            json!({"type":"system","subtype":"init","session_id":"s1","model":"m"}),
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"msg"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"think"}}}),
            json!({"type":"assistant","message":{"id":"msg","content":[{"type":"text","text":"hello"},{"type":"thinking","thinking":"think"},{"type":"tool_use","id":"tool1","name":"Read","input":{"path":"x"}}]}}),
            json!({"type":"assistant","parent_tool_use_id":"sub","message":{"id":"child","content":[{"type":"text","text":"hidden"}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool1","content":"ok"}]}}),
            json!({"type":"result","subtype":"success","session_id":"s1","result":"hello","usage":{"input_tokens":8,"output_tokens":3,"cache_read_input_tokens":2}}),
        ],
    );
    assert_eq!(out.session_id.as_deref(), Some("s1"));
    assert_eq!(out.model.as_deref(), Some("m"));
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
            .filter(|e| matches!(e, HarnessEvent::Reasoning { .. }))
            .count(),
        1
    );
    assert!(events.contains(&HarnessEvent::ToolFinished {
        call_id: "tool1".into(),
        output: "ok".into(),
        is_error: false
    }));
    assert!(events.contains(&HarnessEvent::Usage {
        input_tokens: 8,
        output_tokens: 3,
        cached_input_tokens: 2
    }));
}

#[test]
fn claude_streamed_tool_input_is_complete_and_deduplicated() {
    let (_, events) = parse(
        HarnessKind::ClaudeCode,
        &[
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"Read","input":{}}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"x\"}"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_stop","index":0}}),
            json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"c","name":"Read","input":{"path":"x"}}]}}),
            json!({"type":"result","subtype":"success"}),
        ],
    );
    assert_eq!(
        events,
        vec![HarnessEvent::ToolStarted {
            call_id: "c".into(),
            name: "Read".into(),
            input: json!({"path":"x"})
        }]
    );
}

#[test]
fn claude_fallback_text_permission_warning_and_duplicate_messages() {
    let (_, events) = parse(
        HarnessKind::ClaudeCode,
        &[
            json!({"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"hi"}]}}),
            json!({"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"hi"}]}}),
            json!({"type":"result","subtype":"success","result":"hi","permission_denials":[{}]}),
        ],
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, HarnessEvent::Text { .. }))
            .count(),
        1
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, HarnessEvent::Warning { .. }))
    );
    let (_, events) = parse(
        HarnessKind::ClaudeCode,
        &[json!({"type":"result","subtype":"success","result":"fallback"})],
    );
    assert!(events.contains(&HarnessEvent::Text {
        text: "fallback".into()
    }));
}

#[test]
fn codex_snapshots_tools_usage_and_unknown_events() {
    let (_, events) = parse(
        HarnessKind::Codex,
        &[
            json!({"type":"future.event"}),
            json!({"type":"thread.started","thread_id":"t"}),
            json!({"type":"item.started","item":{"id":"cmd","type":"command_execution","command":"ls"}}),
            json!({"type":"item.completed","item":{"id":"cmd","type":"command_execution","aggregated_output":"oops","exit_code":1}}),
            json!({"type":"item.updated","item":{"id":"m","type":"agent_message","text":"hel"}}),
            json!({"type":"item.completed","item":{"id":"m","type":"agent_message","text":"hello"}}),
            json!({"type":"item.completed","item":{"id":"m","type":"agent_message","text":"hello"}}),
            json!({"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":2,"cached_input_tokens":1}}),
        ],
    );
    let text: String = events
        .iter()
        .filter_map(|e| {
            if let HarnessEvent::Text { text } = e {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(text, "hello");
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, HarnessEvent::ToolStarted { .. }))
            .count(),
        1
    );
    assert!(events.contains(&HarnessEvent::ToolFinished {
        call_id: "cmd".into(),
        output: "oops".into(),
        is_error: true
    }));
}

#[test]
fn claude_same_message_id_can_carry_separate_blocks() {
    let (_, events) = parse(
        HarnessKind::ClaudeCode,
        &[
            json!({"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"answer"}]}}),
            json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"c","name":"Read","input":{}}]}}),
            json!({"type":"result","subtype":"success","result":"answer"}),
        ],
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, HarnessEvent::ToolStarted { call_id, .. } if call_id == "c"))
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, HarnessEvent::Text { .. }))
            .count(),
        1
    );
}

#[test]
fn session_identity_and_terminal_order_are_checked() {
    let mut parser = Parser::new(HarnessKind::Codex);
    parser
        .line(br#"{"type":"thread.started","thread_id":"a"}"#, &mut |_| {})
        .unwrap();
    assert!(
        parser
            .line(br#"{"type":"thread.started","thread_id":"b"}"#, &mut |_| {})
            .is_err()
    );
    let mut parser = Parser::new(HarnessKind::Codex);
    parser
        .line(br#"{"type":"turn.completed","usage":{}}"#, &mut |_| {})
        .unwrap();
    assert!(
        parser
            .line(br#"{"type":"future.event"}"#, &mut |_| {})
            .is_err()
    );
}

#[test]
fn grok_native_protocol_and_cumulative_usage() {
    let (out, events) = parse(
        HarnessKind::GrokBuild,
        &[
            json!({"type":"thought","data":"think"}),
            json!({"type":"text","data":"hello"}),
            json!({"type":"tool_call","toolCallId":"c","toolName":"read_file","rawInput":{"path":"a"}}),
            json!({"type":"tool_call_update","toolCallId":"c","status":"in_progress"}),
            json!({"type":"tool_call_update","toolCallId":"c","status":"completed","rawOutput":"ok"}),
            json!({"type":"usage","usage":{"input_tokens":2,"output_tokens":3}}),
            json!({"type":"usage","usage":{"input_tokens":4,"output_tokens":1}}),
            json!({"type":"end","stopReason":"end_turn","sessionId":"s","model":"grok","usage":{"input_tokens":6,"output_tokens":4}}),
        ],
    );
    assert_eq!(out.session_id.as_deref(), Some("s"));
    assert_eq!(
        events.last(),
        Some(&HarnessEvent::Usage {
            input_tokens: 6,
            output_tokens: 4,
            cached_input_tokens: 0
        })
    );
}

#[test]
fn protocol_failures_are_actionable_and_do_not_expose_payloads() {
    for kind in [
        HarnessKind::ClaudeCode,
        HarnessKind::Codex,
        HarnessKind::GrokBuild,
    ] {
        assert!(Parser::new(kind).finish().is_err());
        let mut p = Parser::new(kind);
        let error = p
            .line(b"secret-token is not json", &mut |_| {})
            .unwrap_err();
        assert!(!error.to_string().contains("secret-token"));
        assert!(p.line(b"[]", &mut |_| {}).is_err());
        assert!(
            p.line(br#"{"type":"error","message":"secret-token"}"#, &mut |_| {})
                .is_err()
        );
    }
    let mut p = Parser::new(HarnessKind::GrokBuild);
    assert!(matches!(
        p.line(br#"{"type":"end","stopReason":"cancelled"}"#, &mut |_| {}),
        Err(HarnessError::Cancelled)
    ));
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    fn script(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{content}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn new_harness_commands_are_detected_independently() {
        let dir = tempfile::tempdir().unwrap();
        for (kind, command) in [
            (HarnessKind::HermesAgent, "hermes"),
            (HarnessKind::OpenClaw, "openclaw"),
            (HarnessKind::OpenCode, "opencode"),
            (HarnessKind::QwenCode, "qwen"),
            (HarnessKind::DeepSeekHarness, "dsh"),
        ] {
            assert!(detect_in(kind, Some(dir.path().as_os_str())).is_none());
            let path = script(dir.path(), command, "exit 0");
            assert_eq!(detect_in(kind, Some(dir.path().as_os_str())), Some(path));
        }
    }

    #[test]
    fn path_detection_requires_executable_and_preserves_launchers() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_in(HarnessKind::Blora, None), Some(PathBuf::new()));
        assert!(detect_in(HarnessKind::Codex, Some(dir.path().as_os_str())).is_none());
        let path = script(dir.path(), "codex", "exit 0");
        assert_eq!(
            detect_in(HarnessKind::Codex, Some(dir.path().as_os_str())),
            Some(path.clone())
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(detect_in(HarnessKind::Codex, Some(dir.path().as_os_str())).is_none());
        std::fs::remove_file(&path).unwrap();
        let launcher = script(dir.path(), "launcher", "exit 0");
        std::os::unix::fs::symlink(&launcher, &path).unwrap();
        assert_eq!(
            detect_in(HarnessKind::Codex, Some(dir.path().as_os_str())),
            Some(path.clone())
        );
    }

    #[test]
    fn symlink_launcher_keeps_argv_zero_when_running_in_another_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let target = script(
            dir.path(),
            "target",
            r#"case "$0" in
*/codex) printf '%s\n' '{"type":"turn.completed","usage":{}}' ;;
*) exit 7 ;;
esac"#,
        );
        let launcher = dir.path().join("codex");
        std::os::unix::fs::symlink(target, &launcher).unwrap();
        let detected = detect_in(HarnessKind::Codex, Some(dir.path().as_os_str())).unwrap();
        assert_eq!(detected, launcher);
        let mut req = request(HarnessKind::Codex, PermissionMode::Ask);
        req.cwd = cwd.path().into();
        execute(&detected, &req, &CancelToken::new(), &mut vec![]).unwrap();
    }

    fn execute(
        path: &Path,
        req: &RunRequest,
        cancel: &CancelToken,
        events: &mut Vec<HarnessEvent>,
    ) -> Result<Outcome> {
        crate::process::execute(path, req, &invocation(req)?, cancel, |e| events.push(e))
    }

    #[test]
    fn fake_cli_streams_preserves_cwd_and_argument_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = script(
            dir.path(),
            "grok",
            "printf '%s\\n' \"$PWD\" > cwd\nprintf '%s\\n' \"$@\" > args\nprintf '%s\\n' '{\"type\":\"text\",\"data\":\"hi\"}'\nsleep 0.02\nprintf '%s' '{\"type\":\"end\",\"stopReason\":\"end_turn\",\"sessionId\":\"s\"}'",
        );
        let mut req = request(HarnessKind::GrokBuild, PermissionMode::Ask);
        req.cwd = dir.path().into();
        let mut events = vec![];
        let out = execute(&path, &req, &CancelToken::new(), &mut events).unwrap();
        assert_eq!(out.session_id.as_deref(), Some("s"));
        assert!(events.contains(&HarnessEvent::Text { text: "hi".into() }));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("cwd"))
                .unwrap()
                .trim(),
            dir.path().to_str().unwrap()
        );
        assert!(
            std::fs::read_to_string(dir.path().join("args"))
                .unwrap()
                .contains(&req.prompt)
        );
        assert!(!dir.path().join("injected").exists());
    }

    #[test]
    fn new_plain_and_json_transports_carry_all_document_output() {
        let dir = tempfile::tempdir().unwrap();
        // Hermes/DeepSeek need not force multi-line answers through a 1 MiB JSONL cap.
        for kind in [HarnessKind::HermesAgent, HarnessKind::DeepSeekHarness] {
            let path = script(
                dir.path(),
                "fake",
                &format!("printf '%s' \"$(printf 'x%.0s' {{1..1100000}})\""),
            );
            let mut req = request(kind, PermissionMode::Yolo);
            req.cwd = dir.path().into();
            let args = invocation(&req).unwrap();
            let mut events = Vec::new();
            crate::process::execute(&path, &req, &args, &CancelToken::new(), |e| events.push(e))
                .unwrap();
            assert!(events.iter().any(|e| matches!(
                e,
                HarnessEvent::Text { text } if text.len() >= 1_100_000
            )));
        }
    }

    #[test]
    fn new_harness_resume_ids_are_restricted_per_transport() {
        for kind in [
            HarnessKind::HermesAgent,
            HarnessKind::OpenClaw,
            HarnessKind::DeepSeekHarness,
        ] {
            let mut req = request(kind, PermissionMode::Yolo);
            req.resume_session_id = Some("any".into());
            assert!(invocation(&req).is_err());
        }
        let mut req = request(HarnessKind::OpenCode, PermissionMode::Ask);
        req.resume_session_id = Some("ses_abc".into());
        assert!(invocation(&req).is_ok());
        for (id, valid) in [
            ("01a10c01-7571-71c2-9f0f-62886fc7be5b", true),
            ("title", false),
        ] {
            req.kind = HarnessKind::QwenCode;
            req.resume_session_id = Some(id.into());
            assert_eq!(invocation(&req).is_ok(), valid);
        }
    }

    #[test]
    fn new_document_transports_parse_only_after_success() {
        let dir = tempfile::tempdir().unwrap();
        for kind in [HarnessKind::HermesAgent, HarnessKind::DeepSeekHarness] {
            let path = script(
                dir.path(),
                "fake",
                "printf '%s\\n' 'answer line one' '{\"text\":\"ordinary reply\"}'",
            );
            let mut req = request(kind, PermissionMode::Yolo);
            req.cwd = dir.path().into();
            let mut events = Vec::new();
            let args = invocation(&req).unwrap();
            let outcome = crate::process::execute(&path, &req, &args, &CancelToken::new(), |e| {
                events.push(e)
            })
            .unwrap();
            assert!(outcome.session_id.is_none());
            assert!(events.iter().any(|e| matches!(e, HarnessEvent::Text { text } if text.contains("answer line one") && text.contains("ordinary reply"))));
            let bad = script(dir.path(), "bad", "printf '%s' 'partial answer'; exit 1");
            events.clear();
            assert!(
                crate::process::execute(&bad, &req, &args, &CancelToken::new(), |e| events.push(e))
                    .is_err()
            );
            assert!(events.is_empty());
        }
        let path = script(
            dir.path(),
            "openclaw-fake",
            "printf '%s\\n' '{' '  \"payloads\": [{\"text\": \"local answer\"}],' '  \"meta\": {\"agentMeta\": {\"sessionId\": \"independent\", \"model\": \"m\"}}' '}'",
        );
        let mut req = request(HarnessKind::OpenClaw, PermissionMode::Yolo);
        req.cwd = dir.path().into();
        let mut events = Vec::new();
        let out =
            crate::process::execute(&path, &req, &[], &CancelToken::new(), |e| events.push(e))
                .unwrap();
        assert_eq!(out.session_id.as_deref(), Some("independent"));
        assert!(events.contains(&HarnessEvent::Text {
            text: "local answer".into()
        }));
    }

    #[test]
    fn fake_claude_and_codex_first_and_resume_rounds() {
        let dir = tempfile::tempdir().unwrap();
        for (kind, body) in [
            (
                HarnessKind::ClaudeCode,
                r#"printf '%s\n' '{"type":"system","session_id":"01a10c01-7571-71c2-9f0f-62886fc7be5b","model":"m"}' '{"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"answer"}]}}' '{"type":"result","subtype":"success","session_id":"01a10c01-7571-71c2-9f0f-62886fc7be5b","result":"answer"}'"#,
            ),
            (
                HarnessKind::Codex,
                r#"printf '%s\n' '{"type":"thread.started","thread_id":"session-1"}' '{"type":"item.completed","item":{"id":"m","type":"agent_message","text":"answer"}}' '{"type":"turn.completed","usage":{"input_tokens":5,"output_tokens":2}}'"#,
            ),
        ] {
            let path = script(
                dir.path(),
                "fake",
                &format!("printf '%s\\n' \"$@\" > args\n{body}"),
            );
            let mut req = request(kind, PermissionMode::Ask);
            req.cwd = dir.path().into();
            for round in 0..2 {
                let mut events = vec![];
                let outcome = execute(&path, &req, &CancelToken::new(), &mut events).unwrap();
                let expected = if kind == HarnessKind::ClaudeCode {
                    "01a10c01-7571-71c2-9f0f-62886fc7be5b"
                } else {
                    "session-1"
                };
                assert_eq!(outcome.session_id.as_deref(), Some(expected));
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| matches!(event, HarnessEvent::Text { .. }))
                        .count(),
                    1
                );
                let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
                assert_eq!(args.contains(expected), round == 1);
                req.resume_session_id = outcome.session_id;
            }
        }
    }

    #[test]
    fn already_cancelled_does_not_spawn() {
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            run(
                &request(HarnessKind::Codex, PermissionMode::Ask),
                &cancel,
                |_| {}
            ),
            Err(HarnessError::Cancelled)
        ));
    }

    #[test]
    fn fake_cli_nonzero_malformed_missing_terminal_and_stderr_flood() {
        let dir = tempfile::tempdir().unwrap();
        let req = request(HarnessKind::Codex, PermissionMode::Ask);
        for content in [
            "printf 'secret-token' >&2; exit 9",
            "echo not-json",
            "echo '{\"type\":\"thread.started\",\"thread_id\":\"t\"}'",
            "head -c 200000 /dev/zero >&2; echo '{\"type\":\"turn.completed\",\"usage\":{}}'; exit 8",
        ] {
            let path = script(dir.path(), "codex", content);
            let error = execute(&path, &req, &CancelToken::new(), &mut vec![]).unwrap_err();
            assert!(!error.to_string().contains("secret-token"));
        }
    }

    #[test]
    fn oversized_stdout_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = script(dir.path(), "codex", "head -c 1100000 /dev/zero");
        assert!(matches!(
            execute(
                &path,
                &request(HarnessKind::Codex, PermissionMode::Ask),
                &CancelToken::new(),
                &mut vec![]
            ),
            Err(HarnessError::Protocol(_))
        ));
    }

    #[test]
    fn cancellation_cleans_descendant_group_without_pipe_hang() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("child.pid");
        let path = script(
            dir.path(),
            "codex",
            &format!("sleep 30 &\necho $! > '{}'\nwait", pid_path.display()),
        );
        let cancel = CancelToken::new();
        let clone = cancel.clone();
        let watcher_path = pid_path.clone();
        let worker = std::thread::spawn(move || {
            let start = Instant::now();
            while !watcher_path.exists() && start.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(10));
            }
            clone.cancel();
        });
        let start = Instant::now();
        assert!(matches!(
            execute(
                &path,
                &request(HarnessKind::Codex, PermissionMode::Ask),
                &cancel,
                &mut vec![]
            ),
            Err(HarnessError::Cancelled)
        ));
        worker.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(3));
        let pid = std::fs::read_to_string(pid_path).unwrap();
        assert_not_running(pid.trim());
    }

    #[test]
    fn exited_leader_with_inherited_pipes_does_not_hang() {
        let dir = tempfile::tempdir().unwrap();
        let path = script(
            dir.path(),
            "codex",
            "sleep 30 &\necho $! > child.pid\necho '{\"type\":\"turn.completed\",\"usage\":{}}'",
        );
        let mut req = request(HarnessKind::Codex, PermissionMode::Ask);
        req.cwd = dir.path().into();
        let start = Instant::now();
        execute(&path, &req, &CancelToken::new(), &mut vec![]).unwrap();
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_not_running(
            std::fs::read_to_string(dir.path().join("child.pid"))
                .unwrap()
                .trim(),
        );
    }

    fn assert_not_running(pid: &str) {
        let start = Instant::now();
        loop {
            match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                Err(_) => return,
                Ok(stat)
                    if stat
                        .split(") ")
                        .nth(1)
                        .is_some_and(|s| s.starts_with('Z') || s.starts_with('X')) =>
                {
                    return;
                }
                _ => {}
            }
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "descendant remains running"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
