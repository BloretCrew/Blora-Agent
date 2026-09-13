// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Compile canonical events into provider-facing messages.
//! Stable bytes stay at the front; workspace facts and transcript follow.

use blora_events::{EventEnvelope, KnownPayload};
use blora_model::{ChatMessage, ToolCall};
use blora_types::Result;

/// Byte-stable identity and safety rules. Do not put dates or cwd here.
#[must_use]
pub fn stable_prompt(mode: &str) -> String {
    format!(
        "You are Blora Agent, a local {mode} harness.\n\
         Stay inside the workspace. Prefer read_file, list_dir, and search before write_file or shell.\n\
         Return a concise final answer when the task is done.\n\
         Do not exfiltrate secrets. Unknown event types in history must be ignored."
    )
}

/// Workspace and environment facts that may change between sessions.
#[must_use]
pub fn context_prompt(workspace: &str, mode: &str) -> String {
    format!(
        "Workspace: {workspace}\n\
         Mode: {mode}\n\
         OS: {}\n\
         Date: {}\n\
         Tools: read_file, write_file, list_dir, search, shell, apply_patch, git_status, git_diff, git_log, git_branch, git_worktree, process, schedule_task, delegate, handoff.",
        std::env::consts::OS,
        chrono::Utc::now().date_naive()
    )
}

#[must_use]
pub fn layered_system_prompt(workspace: &str, mode: &str) -> String {
    format!(
        "[stable]\n{}\n\n[context]\n{}\n\n[volatile]\nFollow the latest user input, tool results, and any compaction summary.",
        stable_prompt(mode),
        context_prompt(workspace, mode)
    )
}

pub fn compile_messages(
    events: &[EventEnvelope],
    workspace: &str,
    mode: &str,
) -> Result<Vec<ChatMessage>> {
    let compact_at = events
        .iter()
        .rposition(|event| event.event_type == "context.compaction.completed");
    let (prefix, rest) = match compact_at {
        Some(index) => {
            let summary = events[index]
                .decode_payload()
                .ok()
                .flatten()
                .and_then(|payload| match payload {
                    KnownPayload::ContextCompactionCompleted(done) => Some(done.summary),
                    _ => None,
                })
                .unwrap_or_else(|| "(compacted)".to_owned());
            (Some(summary), &events[index + 1..])
        }
        None => (None, events),
    };
    let mut messages = vec![ChatMessage {
        role: "system".to_owned(),
        content: Some(layered_system_prompt(workspace, mode)),
        tool_call_id: None,
        tool_calls: None,
    }];
    if let Some(summary) = prefix {
        messages.push(ChatMessage {
            role: "system".to_owned(),
            content: Some(format!("Prior context summary:\n{summary}")),
            tool_call_id: None,
            tool_calls: None,
        });
    }
    let mut assistant_text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    let flush_assistant = |messages: &mut Vec<ChatMessage>,
                           assistant_text: &mut String,
                           tool_calls: &mut Vec<ToolCall>| {
        if assistant_text.is_empty() && tool_calls.is_empty() {
            return;
        }
        messages.push(ChatMessage {
            role: "assistant".to_owned(),
            content: if assistant_text.is_empty() {
                None
            } else {
                Some(std::mem::take(assistant_text))
            },
            tool_call_id: None,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(std::mem::take(tool_calls))
            },
        });
    };

    for event in rest {
        let Some(payload) = event.decode_payload()? else {
            continue;
        };
        match payload {
            KnownPayload::UserInput(input) => {
                flush_assistant(&mut messages, &mut assistant_text, &mut tool_calls);
                messages.push(ChatMessage {
                    role: "user".to_owned(),
                    content: Some(input.text),
                    tool_call_id: None,
                    tool_calls: None,
                });
            }
            KnownPayload::AssistantDelta(delta) => assistant_text.push_str(&delta.text),
            KnownPayload::AssistantMessageCompleted(completed) => {
                if assistant_text.is_empty() {
                    assistant_text = completed.text;
                }
            }
            KnownPayload::ToolRequested(requested) => {
                tool_calls.push(ToolCall {
                    id: requested
                        .call_id
                        .unwrap_or_else(|| format!("call_{}", requested.tool)),
                    name: requested.tool,
                    arguments: requested.arguments.to_string(),
                });
            }
            KnownPayload::ToolOutput(output) => {
                flush_assistant(&mut messages, &mut assistant_text, &mut tool_calls);
                messages.push(ChatMessage {
                    role: "tool".to_owned(),
                    content: Some(output.text),
                    tool_call_id: output.call_id,
                    tool_calls: None,
                });
            }
            _ => {}
        }
    }
    flush_assistant(&mut messages, &mut assistant_text, &mut tool_calls);
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_events::{NewEvent, UserInput};
    use blora_types::{SessionId, SystemClock};

    #[test]
    fn stable_prefix_does_not_include_date() {
        let stable = stable_prompt("code");
        assert!(stable.contains("Blora Agent"));
        assert!(!stable.contains("Date:"));
        assert!(context_prompt("/tmp", "code").contains("Date:"));
    }

    #[test]
    fn compiles_user_input() {
        let session = SessionId::generate();
        let envelope = blora_events::EventEnvelope::from_new(
            NewEvent::new(
                session,
                KnownPayload::UserInput(UserInput {
                    text: "hello".to_owned(),
                }),
            ),
            1,
            &SystemClock,
        )
        .unwrap();
        let messages = compile_messages(&[envelope], "/tmp", "code").unwrap();
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[1].content.as_deref(), Some("hello"));
    }
}
