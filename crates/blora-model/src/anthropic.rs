// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::io::{BufRead, BufReader};

use blora_types::{BloraError, CancelToken, Result};
use serde_json::{Value, json};

use crate::{ChatMessage, Completion, CompletionRequest, Provider, StreamEvent, ToolCall};

pub struct AnthropicProvider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl AnthropicProvider {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .or_else(|_| std::env::var("BLORA_API_KEY"))
            .map_err(|_| BloraError::provider("set ANTHROPIC_API_KEY or BLORA_API_KEY"))?;
        let base_url = std::env::var("BLORA_API_BASE")
            .unwrap_or_else(|_| "https://api.anthropic.com".to_owned());
        let model =
            std::env::var("BLORA_MODEL").unwrap_or_else(|_| "claude-3-5-sonnet-latest".to_owned());
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            model,
        })
    }
}

impl Provider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
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
        let (system, messages) = split_messages(&request.messages);
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                })
            })
            .collect();
        let body = json!({
            "model": model,
            "max_tokens": 4096,
            "stream": true,
            "system": system,
            "messages": messages,
            "tools": tools,
        });
        let url = format!("{}/v1/messages", self.base_url);
        let response = ureq::post(&url)
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("Content-Type", "application/json")
            .set("Accept", "text/event-stream")
            .send_json(body)
            .map_err(|err| BloraError::provider(err.to_string()))?;
        parse_anthropic_sse(BufReader::new(response.into_reader()), cancel, on_event)
    }
}

fn split_messages(messages: &[ChatMessage]) -> (String, Vec<Value>) {
    let mut system = String::new();
    let mut out = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "system" => {
                if let Some(content) = &message.content {
                    if !system.is_empty() {
                        system.push('\n');
                    }
                    system.push_str(content);
                }
            }
            "tool" => out.push(json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": message.tool_call_id,
                    "content": message.content,
                }]
            })),
            "assistant" => {
                let mut content = Vec::new();
                if let Some(text) = &message.content {
                    content.push(json!({"type": "text", "text": text}));
                }
                if let Some(calls) = &message.tool_calls {
                    for call in calls {
                        let input = serde_json::from_str::<Value>(&call.arguments)
                            .unwrap_or_else(|_| json!({"raw": call.arguments}));
                        content.push(json!({
                            "type": "tool_use",
                            "id": call.id,
                            "name": call.name,
                            "input": input,
                        }));
                    }
                }
                out.push(json!({"role": "assistant", "content": content}));
            }
            _ => out.push(json!({
                "role": "user",
                "content": message.content,
            })),
        }
    }
    (system, out)
}

pub fn parse_anthropic_sse(
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
        if data.is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(data).map_err(|err| BloraError::provider(err.to_string()))?;
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "content_block_start" => {
                if let Some(block) = value.get("content_block") {
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        pending.id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        pending.name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                    }
                }
            }
            "content_block_delta" => {
                if let Some(delta) = value.get("delta") {
                    match delta.get("type").and_then(Value::as_str) {
                        Some("text_delta") => {
                            if let Some(text) = delta.get("text").and_then(Value::as_str) {
                                completion.text.push_str(text);
                                on_event(StreamEvent::TextDelta(text.to_owned()))?;
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(partial) = delta.get("partial_json").and_then(Value::as_str)
                            {
                                pending.arguments.push_str(partial);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "message_delta" => {
                if let Some(stop) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    completion.finish_reason = stop.to_owned();
                }
                if let Some(usage) = value.get("usage") {
                    completion.output_tokens = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(completion.output_tokens);
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
        if completion.finish_reason.is_empty() {
            completion.finish_reason = "tool_use".to_owned();
        }
    } else if completion.finish_reason.is_empty() {
        completion.finish_reason = "end_turn".to_owned();
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
    fn parses_text_and_tool_use() {
        let body = "\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\
data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"search\"}}\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"pattern\\\":\\\"fn\\\"}\"}}\n";
        let completion =
            parse_anthropic_sse(Cursor::new(body), &CancelToken::new(), &mut |_| Ok(())).unwrap();
        assert_eq!(completion.text, "ok");
        assert_eq!(completion.tool_calls[0].name, "search");
    }
}
