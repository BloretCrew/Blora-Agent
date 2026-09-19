// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};

use blora_types::{BloraError, CancelToken, Result};
use serde_json::{Value, json};

use crate::{ChatMessage, Completion, CompletionRequest, Provider, StreamEvent, ToolCall, http};

/// Anthropic accepts at most four cache breakpoints per request.
const MAX_BREAKPOINTS: usize = 4;

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
    fn name(&self) -> &str {
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
        let (system, messages) = build_messages(&request.messages);
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
        let mut body = json!({
            "model": model,
            "max_tokens": request.max_output_tokens.unwrap_or_else(http::max_output_tokens),
            "stream": true,
            "messages": messages,
        });
        if !system.is_empty() {
            body["system"] = Value::Array(system);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        if let Some(key) = &request.cache_key {
            body["metadata"] = json!({ "user_id": key });
        }
        let url = format!("{}/v1/messages", self.base_url);
        let response = http::agent()
            .post(&url)
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("Content-Type", "application/json")
            .set("Accept", "text/event-stream")
            .send_json(body)
            .map_err(http::map_error)?;
        parse_anthropic_sse(BufReader::new(response.into_reader()), cancel, on_event)
    }
}

fn cache_control() -> Value {
    json!({"type": "ephemeral"})
}

/// Compile canonical messages into Anthropic `system` blocks and alternating messages.
///
/// Consecutive same-role turns are merged, tool results become `tool_result` blocks,
/// and cache breakpoints are honoured up to the API limit: system blocks first, then
/// the most recent flagged user turns.
pub fn build_messages(messages: &[ChatMessage]) -> (Vec<Value>, Vec<Value>) {
    let mut system = Vec::new();
    let mut out: Vec<(String, Vec<Value>)> = Vec::new();
    let mut breakpoints = 0usize;
    let mut message_breakpoints: Vec<(usize, usize)> = Vec::new();

    for message in messages {
        match message.role.as_str() {
            "system" => {
                let Some(text) = message.content.as_deref().filter(|t| !t.is_empty()) else {
                    continue;
                };
                let mut block = json!({"type": "text", "text": text});
                if message.cache_breakpoint && breakpoints < MAX_BREAKPOINTS {
                    block["cache_control"] = cache_control();
                    breakpoints += 1;
                }
                system.push(block);
            }
            "tool" => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.content.clone().unwrap_or_default(),
                });
                push_block(
                    &mut out,
                    "user",
                    block,
                    message.cache_breakpoint,
                    &mut message_breakpoints,
                );
            }
            "assistant" => {
                let mut blocks = Vec::new();
                if let Some(text) = message.content.as_deref().filter(|t| !t.trim().is_empty()) {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                if let Some(calls) = &message.tool_calls {
                    for call in calls {
                        let input = serde_json::from_str::<Value>(&call.arguments)
                            .unwrap_or_else(|_| json!({"raw": call.arguments}));
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": call.id,
                            "name": call.name,
                            "input": input,
                        }));
                    }
                }
                for block in blocks {
                    push_block(
                        &mut out,
                        "assistant",
                        block,
                        false,
                        &mut message_breakpoints,
                    );
                }
            }
            _ => {
                let text = message.content.clone().unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                let block = json!({"type": "text", "text": text});
                push_block(
                    &mut out,
                    "user",
                    block,
                    message.cache_breakpoint,
                    &mut message_breakpoints,
                );
            }
        }
    }

    // Apply the most recent message-level breakpoints within the remaining budget.
    let remaining = MAX_BREAKPOINTS.saturating_sub(breakpoints);
    for (message_index, block_index) in message_breakpoints.iter().rev().take(remaining) {
        if let Some((_, blocks)) = out.get_mut(*message_index) {
            if let Some(block) = blocks.get_mut(*block_index) {
                block["cache_control"] = cache_control();
            }
        }
    }

    let messages = out
        .into_iter()
        .filter(|(_, blocks)| !blocks.is_empty())
        .map(|(role, blocks)| json!({"role": role, "content": blocks}))
        .collect();
    (system, messages)
}

fn push_block(
    out: &mut Vec<(String, Vec<Value>)>,
    role: &str,
    block: Value,
    breakpoint: bool,
    breakpoints: &mut Vec<(usize, usize)>,
) {
    match out.last_mut() {
        Some((last_role, blocks)) if last_role.as_str() == role => {
            blocks.push(block);
        }
        _ => out.push((role.to_owned(), vec![block])),
    }
    if breakpoint {
        let message_index = out.len() - 1;
        let block_index = out[message_index].1.len() - 1;
        breakpoints.push((message_index, block_index));
    }
}

pub fn parse_anthropic_sse(
    reader: impl BufRead,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
) -> Result<Completion> {
    let mut completion = Completion::default();
    let mut pending: BTreeMap<u64, PartialCall> = BTreeMap::new();
    let mut terminated = false;
    let mut saw_event = false;
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
            "ping" => {}
            "error" => {
                let status = value
                    .pointer("/error/type")
                    .and_then(Value::as_str)
                    .map(|t| if t == "overloaded_error" { 529 } else { 500 })
                    .unwrap_or(500);
                return Err(BloraError::provider(format!(
                    "HTTP {status}: in-stream error {}",
                    value.get("error").cloned().unwrap_or(Value::Null)
                )));
            }
            "message_start" => {
                saw_event = true;
                if let Some(usage) = value.pointer("/message/usage") {
                    let input = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let cached = usage
                        .get("cache_read_input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let created = usage
                        .get("cache_creation_input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    // `input_tokens` excludes cached prefixes; report the full prompt size.
                    completion.input_tokens = input + cached + created;
                    completion.cached_tokens = cached;
                }
            }
            "content_block_start" => {
                saw_event = true;
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                if let Some(block) = value.get("content_block") {
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        pending.insert(
                            index,
                            PartialCall {
                                id: block
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                                name: block
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                                arguments: String::new(),
                            },
                        );
                    }
                }
            }
            "content_block_delta" => {
                saw_event = true;
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
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
                                pending
                                    .entry(index)
                                    .or_default()
                                    .arguments
                                    .push_str(partial);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "message_delta" => {
                saw_event = true;
                if let Some(stop) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    completion.finish_reason = stop.to_owned();
                    terminated = true;
                }
                if let Some(usage) = value.get("usage") {
                    completion.output_tokens = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(completion.output_tokens);
                }
            }
            "message_stop" => {
                terminated = true;
            }
            _ => {}
        }
    }
    if !terminated && !saw_event {
        return Err(http::incomplete_stream("message_start"));
    }
    for (_, partial) in pending {
        if partial.name.is_empty() {
            continue;
        }
        let call = ToolCall {
            id: partial.id,
            name: partial.name,
            arguments: if partial.arguments.trim().is_empty() {
                "{}".to_owned()
            } else {
                partial.arguments
            },
        };
        on_event(StreamEvent::ToolCall(call.clone()))?;
        completion.tool_calls.push(call);
    }
    if !completion.tool_calls.is_empty() {
        completion.finish_reason = "tool_use".to_owned();
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
    fn parses_text_and_multiple_tool_uses() {
        let body = "\
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5,\"cache_read_input_tokens\":100}}}\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"search\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"pattern\\\":\\\"fn\\\"}\"}}\n\
data: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t2\",\"name\":\"list_dir\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":9}}\n\
data: {\"type\":\"message_stop\"}\n";
        let completion =
            parse_anthropic_sse(Cursor::new(body), &CancelToken::new(), &mut |_| Ok(())).unwrap();
        assert_eq!(completion.text, "ok");
        assert_eq!(completion.tool_calls.len(), 2);
        assert_eq!(completion.tool_calls[0].name, "search");
        assert_eq!(completion.tool_calls[1].id, "t2");
        assert_eq!(completion.cached_tokens, 100);
        assert_eq!(completion.input_tokens, 105);
        assert_eq!(completion.output_tokens, 9);
    }

    #[test]
    fn builds_alternating_messages_with_breakpoints() {
        let messages = vec![
            ChatMessage::text("system", "stable").with_breakpoint(),
            ChatMessage::text("system", "context").with_breakpoint(),
            ChatMessage::text("user", "hi"),
            ChatMessage::assistant(
                None,
                vec![ToolCall {
                    id: "c1".to_owned(),
                    name: "list_dir".to_owned(),
                    arguments: "{}".to_owned(),
                }],
            ),
            ChatMessage::tool_result("c1", "a\nb").with_breakpoint(),
            ChatMessage::text("user", "thanks").with_breakpoint(),
        ];
        let (system, out) = build_messages(&messages);
        assert_eq!(system.len(), 2);
        assert!(system[1].get("cache_control").is_some());
        // user(hi) / assistant(tool_use) / user(tool_result + text) => strict alternation.
        assert_eq!(out.len(), 3);
        assert_eq!(out[2]["role"], "user");
        let blocks = out[2]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].get("cache_control").is_some());
        assert!(blocks[1].get("cache_control").is_some());
        let total = system
            .iter()
            .chain(
                out.iter()
                    .flat_map(|m| m["content"].as_array().unwrap().iter()),
            )
            .filter(|b| b.get("cache_control").is_some())
            .count();
        assert!(total <= MAX_BREAKPOINTS);
    }
}
