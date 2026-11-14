// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use blora_model::{Completion, MockProvider, ToolCall};
use blora_sdk::{
    Agent, ApprovalPolicy, KnownPayload, Profile, ProviderConfig, RunStatus, SdkError, Storage,
    ToolOutcome, tool,
};
use serde_json::json;

fn walk(root: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            out.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
            );
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    out
}

fn mock_with(completions: Vec<Completion>) -> Arc<MockProvider> {
    let provider = Arc::new(MockProvider::new());
    for completion in completions {
        provider.push(completion);
    }
    provider
}

fn text_completion(text: &str) -> Completion {
    Completion {
        text: text.to_owned(),
        finish_reason: "stop".to_owned(),
        ..Completion::default()
    }
}

fn tool_completion(name: &str, arguments: serde_json::Value) -> Completion {
    Completion {
        tool_calls: vec![ToolCall {
            id: format!("call_{name}"),
            name: name.to_owned(),
            arguments: arguments.to_string(),
        }],
        finish_reason: "tool_calls".to_owned(),
        ..Completion::default()
    }
}

#[test]
fn basic_chat_completes() {
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_with(vec![text_completion("Hello from SDK")]);
    let agent = Agent::builder()
        .profile(Profile::full())
        .workspace(dir.path())
        .storage(Storage::memory())
        .provider(ProviderConfig::custom(provider))
        .auto_title(false)
        .build()
        .unwrap();
    let session = agent.create_session().unwrap();
    let outcome = session.run_blocking("hello").unwrap();
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(outcome.final_text, "Hello from SDK");
}

#[test]
fn provider_is_required() {
    let result = Agent::builder().profile(Profile::web()).build();
    assert!(matches!(result, Err(SdkError::ProviderRequired)));
}

#[test]
fn web_profile_rejects_write_file() {
    let provider = mock_with(vec![
        tool_completion(
            "write_file",
            json!({"path": "owned.txt", "contents": "should not exist"}),
        ),
        text_completion("done"),
    ]);
    let agent = Agent::builder()
        .profile(Profile::web())
        .provider(ProviderConfig::custom(provider))
        .build()
        .unwrap();
    let session = agent.create_session().unwrap();
    let outcome = session.run_blocking("write a file").unwrap();
    assert_eq!(outcome.status, RunStatus::Completed);

    let mut stream = session.events_from(0);
    let mut failure = None;
    while let Some(event) = stream.next_event().unwrap() {
        if event.event_type == "tool.failed"
            && let Ok(Some(KnownPayload::ToolFailed(payload))) =
                KnownPayload::from_type(&event.event_type, &event.payload)
        {
            failure = Some(payload);
        }
    }
    let failure = failure.expect("write_file should have failed");
    assert_eq!(failure.tool, "write_file");
    assert!(failure.error.contains("not enabled"), "{}", failure.error);
    let entries: Vec<_> = walk(agent.workspace());
    assert!(
        !agent.workspace().join(".blora").exists(),
        "unexpected workspace entries: {entries:?}"
    );
    let projection = session.projection().unwrap();
    assert!(projection.transcript.iter().any(
        |item| matches!(item, blora_sdk::TranscriptItem::Tool { name, status, .. } if name == "write_file" && status == "failed")
    ));
}

#[test]
fn custom_tool_runs_and_returns_text() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let echo = tool(
        "echo",
        "Echo the provided text",
        json!({
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"]
        }),
        true,
        move |_ctx, arguments| {
            counter.fetch_add(1, Ordering::SeqCst);
            let text = arguments
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            Ok(ToolOutcome::text(text))
        },
    );
    let provider = mock_with(vec![
        tool_completion("echo", json!({"text": "hi"})),
        text_completion("after tool"),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let agent = Agent::builder()
        .profile(Profile::full())
        .workspace(dir.path())
        .provider(ProviderConfig::custom(provider))
        .tool(echo)
        .auto_title(false)
        .build()
        .unwrap();
    let session = agent.create_session().unwrap();
    let outcome = session.run_blocking("use echo").unwrap();
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(outcome.final_text, "after tool");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn approval_deny_blocks_custom_tool() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let dangerous = tool(
        "dangerous",
        "A tool that needs approval",
        json!({"type": "object", "properties": {}}),
        false,
        move |_ctx, _arguments| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutcome::text("executed"))
        },
    );
    let provider = mock_with(vec![
        tool_completion("dangerous", json!({})),
        text_completion("denied"),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let agent = Agent::builder()
        .profile(Profile::full())
        .workspace(dir.path())
        .provider(ProviderConfig::custom(provider))
        .tool(dangerous)
        .approval_policy(ApprovalPolicy::DenyAll)
        .auto_title(false)
        .build()
        .unwrap();
    let session = agent.create_session().unwrap();
    let outcome = session.run_blocking("run dangerous").unwrap();
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(outcome.final_text, "denied");
}
