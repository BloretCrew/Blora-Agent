// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::io::{BufRead, BufReader};

use blora_types::{BloraError, CancelToken, Result};
use serde_json::{Value, json};

use crate::{ChatMessage, Completion, CompletionRequest, Provider, StreamEvent, ToolCall};

pub struct ResponsesProvider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl ResponsesProvider {
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

impl Provider for ResponsesProvider {
    fn name(&self) -> &'static str {
        "responses"
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
        let (instructions, input) = split_input(&request.messages);
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                })
            })
            .collect();
        let body = json!({
            "model": model,
            "stream": true,
            "instructions": instructions,
            "input": input,
            "tools": tools,
        });
        let url = format!("{}/responses", self.base_url);
        let response = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .set("Accept", "text/event-stream")
            .send_json(body)
            .map_err(|err| BloraError::provider(err.to_string()))?;
        parse_responses_sse(BufReader::new(response.into_reader()), cancel, on_event)
    }
}

fn split_input(messages: &[ChatMessage]) -> (String, Vec<Value>) {
    let mut instructions = String::new();
    let mut input = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "system" => {
                if let Some(content) = &message.content {
                    if !instructions.is_empty() {
                        instructions.push('\n');
                    }
                    instructions.push_str(content);
                }
            }
            "tool" => input.push(json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id,
                "output": message.content,
            })),
            _ => {
                if let Some(calls) = &message.tool_calls {
                    for call in calls {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": call.arguments,
                        }));
                    }
                }
                if let Some(content) = &message.content {
                    input.push(json!({
                        "role": message.role,
                        "content": content,
                    }));
                }
            }
        }
    }
    (instructions, input)
}

pub fn parse_responses_sse(
    reader: impl BufRead,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
) -> Result<Completion> {
    let mut completion = Completion::default();
    let mut pending = PartialCall::default();
    for line in reader.lines() {
        if cancel.is_cancelled() {
            return Err(BloraError::Cancelled);
        }
        let line = line.map_err(BloraError::provider)?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let value: Value =
            serde_json::from_str(data).map_err(|err| BloraError::provider(err.to_string()))?;
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "response.output_text.delta" => {
                if let Some(text) = value.get("delta").and_then(Value::as_str) {
                    completion.text.push_str(text);
                    on_event(StreamEvent::TextDelta(text.to_owned()))?;
                }
            }
            "response.output_item.added" => {
                if let Some(item) = value.get("item") {
                    if item.get("type").and_then(Value::as_str) == Some("function_call") {
                        pending.id = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        pending.name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    pending.arguments.push_str(delta);
                }
            }
            "response.completed" => {
                if let Some(usage) = value.pointer("/response/usage") {
                    completion.input_tokens = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    completion.output_tokens = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                }
            }
            _ => {}
        }
    }
    if !pending.name.is_empty() {
        let call = ToolCall {
            id: pending.id,
            name: pending.name,
            arguments: pending.arguments,
        };
        on_event(StreamEvent::ToolCall(call.clone()))?;
        completion.tool_calls.push(call);
        completion.finish_reason = "tool_calls".to_owned();
    } else if completion.finish_reason.is_empty() {
        completion.finish_reason = "stop".to_owned();
    }
    Ok(completion)
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_text_and_function_call() {
        let body = "\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\
data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"call_id\":\"c1\",\"name\":\"list_dir\"}}\n\
data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{\\\"path\\\":\\\".\\\"}\"}\n\
data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n";
        let mut deltas = Vec::new();
        let completion =
            parse_responses_sse(Cursor::new(body), &CancelToken::new(), &mut |event| {
                if let StreamEvent::TextDelta(text) = event {
                    deltas.push(text);
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(deltas, vec!["Hi".to_owned()]);
        assert_eq!(completion.tool_calls[0].name, "list_dir");
        assert_eq!(completion.input_tokens, 3);
    }
}
