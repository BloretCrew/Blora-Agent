// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::io::{BufRead, BufReader};

use blora_types::{BloraError, CancelToken, Result};
use serde_json::{Value, json};

use crate::{ChatMessage, Completion, CompletionRequest, Provider, StreamEvent, ToolCall, http};

fn openai_messages(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            let mut value = json!({ "role": message.role });
            if let Some(content) = &message.content {
                value["content"] = json!(content);
            } else if message.role == "assistant" {
                value["content"] = Value::Null;
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
    pub name: String,
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
            name: "openai".to_owned(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            model,
        })
    }

    #[must_use]
    pub fn with_identity(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }
}

/// Build the Chat Completions request body for `model`.
pub(crate) fn chat_completions_body(request: &CompletionRequest, model: &str) -> Value {
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
    let mut body = json!({
        "model": model,
        "stream": true,
        "stream_options": {"include_usage": true},
        "messages": openai_messages(&request.messages),
        "max_completion_tokens": request.max_output_tokens.unwrap_or_else(http::max_output_tokens),
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    if let Some(key) = &request.cache_key {
        body["prompt_cache_key"] = json!(key);
    }
    body
}

/// POST a Chat Completions body and return the SSE reader.
pub(crate) fn post_chat_completions(
    base_url: &str,
    api_key: &str,
    body: Value,
) -> Result<impl BufRead> {
    let url = format!("{base_url}/chat/completions");
    let response = http::agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream")
        .send_json(body)
        .map_err(http::map_error)?;
    Ok(BufReader::new(response.into_reader()))
}

impl Provider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.name
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
        let body = chat_completions_body(request, &model);
        let reader = post_chat_completions(&self.base_url, &self.api_key, body)?;
        parse_sse(reader, cancel, on_event)
    }
}

pub fn parse_sse(
    reader: impl BufRead,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
) -> Result<Completion> {
    let mut completion = Completion::default();
    let mut pending: Vec<PartialCall> = Vec::new();
    let mut terminated = false;
    let mut saw_chunk = false;
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
            terminated = true;
            break;
        }
        let value: Value =
            serde_json::from_str(data).map_err(|err| BloraError::provider(err.to_string()))?;
        if let Some(error) = value.get("error") {
            return Err(BloraError::provider(format!(
                "HTTP 500: in-stream error {error}"
            )));
        }
        saw_chunk = true;
        if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
            completion.input_tokens = usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(completion.input_tokens);
            completion.output_tokens = usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(completion.output_tokens);
            completion.cached_tokens = usage
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(completion.cached_tokens);
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
                terminated = true;
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
        if let Some(reasoning) = delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            on_event(StreamEvent::ReasoningDelta(reasoning.to_owned()))?;
        }
        if delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
            .is_some_and(Value::is_null)
        {
            on_event(StreamEvent::ReasoningComplete)?;
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                while pending.len() <= index {
                    pending.push(PartialCall::default());
                }
                let slot = &mut pending[index];
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    // Some gateways resend the id on every chunk; assignment, never append.
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
    if !terminated && !saw_chunk {
        return Err(http::incomplete_stream("any chat completion chunk"));
    }
    for (index, partial) in pending.into_iter().enumerate() {
        if partial.name.is_empty() {
            continue;
        }
        let call = ToolCall {
            id: if partial.id.is_empty() {
                format!("call_{}_{index}", partial.name)
            } else {
                partial.id
            },
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
    fn parses_parallel_tool_calls_and_cached_usage() {
        let body = "\
data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"b\",\"function\":{\"name\":\"list_dir\",\"arguments\":\"{}\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"x\\\"}\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"prompt_tokens_details\":{\"cached_tokens\":6}}}\n\
data: [DONE]\n";
        let completion =
            parse_sse(Cursor::new(body), &CancelToken::new(), &mut |_| Ok(())).unwrap();
        assert_eq!(completion.tool_calls.len(), 2);
        assert_eq!(completion.tool_calls[0].arguments, "{\"path\":\"x\"}");
        assert_eq!(completion.tool_calls[1].name, "list_dir");
        assert_eq!(completion.cached_tokens, 6);
        assert_eq!(completion.finish_reason, "tool_calls");
    }

    #[test]
    fn empty_stream_is_an_error() {
        let err = parse_sse(Cursor::new(""), &CancelToken::new(), &mut |_| Ok(())).unwrap_err();
        assert!(err.to_string().contains("stream ended before"));
    }
}
