// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Provider-independent completion types plus protocol adapters.

mod anthropic;
mod factory;
mod gemini;
mod openai;
mod responses;

use std::collections::VecDeque;
use std::sync::Mutex;

use blora_types::{BloraError, CancelToken, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use anthropic::AnthropicProvider;
pub use factory::{make_provider, make_providers};
pub use gemini::GeminiProvider;
pub use openai::OpenAiProvider;
pub use responses::ResponsesProvider;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDeclaration>,
}

#[derive(Clone, Debug, Default)]
pub struct Completion {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub finish_reason: String,
}

#[derive(Clone, Debug)]
pub enum StreamEvent {
    TextDelta(String),
    ToolCall(ToolCall),
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    fn complete(
        &self,
        request: &CompletionRequest,
        cancel: &CancelToken,
        on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
    ) -> Result<Completion>;
}

pub struct MockProvider {
    scripted: Mutex<VecDeque<Completion>>,
}

impl MockProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            scripted: Mutex::new(VecDeque::new()),
        }
    }

    pub fn push(&self, completion: Completion) {
        self.scripted.lock().unwrap().push_back(completion);
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn complete(
        &self,
        request: &CompletionRequest,
        cancel: &CancelToken,
        on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
    ) -> Result<Completion> {
        if cancel.is_cancelled() {
            return Err(BloraError::Cancelled);
        }
        if let Some(scripted) = self.scripted.lock().unwrap().pop_front() {
            emit_completion(&scripted, on_event)?;
            return Ok(scripted);
        }
        let last_role = request.messages.last().map(|message| message.role.as_str());
        let system = request
            .messages
            .iter()
            .find(|message| message.role == "system")
            .and_then(|message| message.content.as_deref())
            .unwrap_or_default();
        let last_user = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .and_then(|message| message.content.clone())
            .unwrap_or_default();
        let completion = if last_role == Some("tool") {
            let listing = request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == "tool")
                .and_then(|message| message.content.clone())
                .unwrap_or_default();
            let text = format!(
                "Mock provider finished after tools.\n\n{}\n\nSet BLORA_API_KEY to use a real model.",
                truncate(&listing, 1500)
            );
            Completion {
                text,
                finish_reason: "stop".to_owned(),
                ..Completion::default()
            }
        } else if last_user.contains("WRITE_FILE") {
            Completion {
                tool_calls: vec![ToolCall {
                    id: "call_mock_write".to_owned(),
                    name: "write_file".to_owned(),
                    arguments: json!({"path": "ok.txt", "contents": "ok"}).to_string(),
                }],
                finish_reason: "tool_calls".to_owned(),
                ..Completion::default()
            }
        } else if last_user.contains("[background-task]")
            || last_user.contains("[subagent:")
            || last_user.contains("Do not spawn another subagent")
        {
            Completion {
                tool_calls: vec![ToolCall {
                    id: "call_mock_list".to_owned(),
                    name: "list_dir".to_owned(),
                    arguments: json!({"path": "."}).to_string(),
                }],
                finish_reason: "tool_calls".to_owned(),
                ..Completion::default()
            }
        } else if system.contains("local agent harness")
            || last_user.to_ascii_lowercase().contains("delegate")
            || last_user.to_ascii_lowercase().contains("subagent")
        {
            Completion {
                tool_calls: vec![ToolCall {
                    id: "call_mock_delegate".to_owned(),
                    name: "delegate".to_owned(),
                    arguments: json!({
                        "role": "worker",
                        "prompt": last_user
                    })
                    .to_string(),
                }],
                finish_reason: "tool_calls".to_owned(),
                ..Completion::default()
            }
        } else if system.contains("local work harness") {
            Completion {
                tool_calls: vec![ToolCall {
                    id: "call_mock_schedule".to_owned(),
                    name: "schedule_task".to_owned(),
                    arguments: json!({
                        "title": "follow-up",
                        "prompt": last_user,
                        "delay_seconds": 0
                    })
                    .to_string(),
                }],
                finish_reason: "tool_calls".to_owned(),
                ..Completion::default()
            }
        } else {
            Completion {
                tool_calls: vec![ToolCall {
                    id: "call_mock_list".to_owned(),
                    name: "list_dir".to_owned(),
                    arguments: json!({"path": "."}).to_string(),
                }],
                finish_reason: "tool_calls".to_owned(),
                ..Completion::default()
            }
        };
        emit_completion(&completion, on_event)?;
        Ok(completion)
    }
}

fn emit_completion(
    completion: &Completion,
    on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
) -> Result<()> {
    if !completion.text.is_empty() {
        on_event(StreamEvent::TextDelta(completion.text.clone()))?;
    }
    for call in &completion.tool_calls {
        on_event(StreamEvent::ToolCall(call.clone()))?;
    }
    Ok(())
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_owned()
    } else {
        format!("{}…", &text[..max])
    }
}

pub fn system_prompt(workspace: &str, mode: &str) -> String {
    format!(
        "You are Blora Agent, a local {mode} harness.\n\
         Workspace: {workspace}\n\
         Stay inside the workspace. Prefer read_file, list_dir, and search before write_file or shell.\n\
         Return a concise final answer when the task is done."
    )
}
