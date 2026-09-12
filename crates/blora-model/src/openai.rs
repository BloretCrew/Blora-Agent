// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::io::{BufRead, BufReader};

use blora_types::{BloraError, CancelToken, Result};
use serde_json::{Value, json};

use crate::{ChatMessage, Completion, CompletionRequest, Provider, StreamEvent, ToolCall};

fn openai_messages(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            let mut value = json!({ "role": message.role });
            if let Some(content) = &message.content {
                value["content"] = json!(content);
            }
            if let Some(tool_call_id) = &message.tool_call_id {
                value["tool_call_id"] = json!(tool_call_id);
            }
            if let Some(calls) = &message.tool_calls {
                value["tool_calls"] = Value::Array(
                    calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": call.arguments,
                                }
                            })
                        })
                        .collect(),
                );
            }
            value
        })
        .collect()
}

pub struct OpenAiProvider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl OpenAiProvider {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("BLORA_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .map_err(|_| BloraError::provider("set BLORA_API_KEY or OPENAI_API_KEY"))?;
        let base_url = std::env::var("BLORA_API_BASE")
            .or_else(|_| std::env::var("OPENAI_BASE_URL"))
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
        let model = std::env::var("BLORA_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_owned());
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            model,
        })
    }
}

impl Provider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai"
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
        let model = if request.model.is_empty() {
            self.model.clone()
        } else {
            request.model.clone()
        };
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect();
        let body = json!({
            "model": model,
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": openai_messages(&request.messages),
            "tools": tools,
        });
        let url = format!("{}/chat/completions", self.base_url);
        let response = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .set("Accept", "text/event-stream")
            .send_json(body)
            .map_err(|err| BloraError::provider(err.to_string()))?;
        let reader = BufReader::new(response.into_reader());
        parse_sse(reader, cancel, on_event)
    }
}

fn parse_sse(
    reader: impl BufRead,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
) -> Result<Completion> {
    let mut completion = Completion::default();
    let mut pending: Vec<PartialCall> = Vec::new();
    for line in reader.lines() {
        if cancel.is_cancelled() {
            return Err(BloraError::Cancelled);
        }
        let line = line.map_err(BloraError::provider)?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            break;
        }
        let value: Value =
            serde_json::from_str(data).map_err(|err| BloraError::provider(err.to_string()))?;
        if let Some(usage) = value.get("usage") {
            completion.input_tokens = usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            completion.output_tokens = usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
        }
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
        else {
            continue;
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            if !reason.is_empty() && reason != "null" {
                completion.finish_reason = reason.to_owned();
            }
        }
        let Some(delta) = choice.get("delta") else {
            continue;
        };
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                completion.text.push_str(text);
                on_event(StreamEvent::TextDelta(text.to_owned()))?;
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                while pending.len() <= index {
                    pending.push(PartialCall::default());
                }
                let slot = &mut pending[index];
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    slot.id = id.to_owned();
                }
                if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                    slot.name = name.to_owned();
                }
                if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str)
                {
                    slot.arguments.push_str(arguments);
                }
            }
        }
    }
    for partial in pending {
        if partial.name.is_empty() {
            continue;
        }
        let call = ToolCall {
            id: if partial.id.is_empty() {
                format!("call_{}", partial.name)
            } else {
                partial.id
            },
            name: partial.name,
            arguments: partial.arguments,
        };
        on_event(StreamEvent::ToolCall(call.clone()))?;
        completion.tool_calls.push(call);
    }
    if completion.finish_reason.is_empty() {
        completion.finish_reason = if completion.tool_calls.is_empty() {
            "stop".to_owned()
        } else {
            "tool_calls".to_owned()
        };
    }
    Ok(completion)
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}
