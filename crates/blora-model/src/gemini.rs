// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::io::{BufRead, BufReader};

use blora_types::{BloraError, CancelToken, Result};
use serde_json::{Value, json};

use crate::{ChatMessage, Completion, CompletionRequest, Provider, StreamEvent, ToolCall};

pub struct GeminiProvider {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

impl GeminiProvider {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .or_else(|_| std::env::var("BLORA_API_KEY"))
            .map_err(|_| BloraError::provider("set GEMINI_API_KEY or BLORA_API_KEY"))?;
        let base_url = std::env::var("BLORA_API_BASE")
            .unwrap_or_else(|_| "https://generativelanguage.googleapis.com".to_owned());
        let model = std::env::var("BLORA_MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_owned());
        Ok(Self {
            api_key,
            model,
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }
}

impl Provider for GeminiProvider {
    fn name(&self) -> &'static str {
        "gemini"
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
        let (system, contents) = split_contents(&request.messages);
        let declarations: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                })
            })
            .collect();
        let mut body = json!({
            "contents": contents,
            "tools": [{ "functionDeclarations": declarations }],
        });
        if !system.is_empty() {
            body["systemInstruction"] = json!({ "parts": [{ "text": system }] });
        }
        let url = format!(
            "{}/v1beta/models/{model}:streamGenerateContent?alt=sse&key={}",
            self.base_url, self.api_key
        );
        let response = ureq::post(&url)
            .set("Content-Type", "application/json")
            .set("Accept", "text/event-stream")
            .send_json(body)
            .map_err(|err| BloraError::provider(err.to_string()))?;
        parse_gemini_sse(BufReader::new(response.into_reader()), cancel, on_event)
    }
}

fn split_contents(messages: &[ChatMessage]) -> (String, Vec<Value>) {
    let mut system = String::new();
    let mut contents = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "system" => {
                if let Some(text) = &message.content {
                    if !system.is_empty() {
                        system.push('\n');
                    }
                    system.push_str(text);
                }
            }
            "assistant" => {
                let mut parts = Vec::new();
                if let Some(text) = &message.content {
                    parts.push(json!({"text": text}));
                }
                if let Some(calls) = &message.tool_calls {
                    for call in calls {
                        let args = serde_json::from_str::<Value>(&call.arguments)
                            .unwrap_or_else(|_| json!({"raw": call.arguments}));
                        parts.push(json!({"functionCall": {"name": call.name, "args": args}}));
                    }
                }
                contents.push(json!({"role": "model", "parts": parts}));
            }
            "tool" => contents.push(json!({
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "name": message.tool_call_id,
                        "response": { "output": message.content },
                    }
                }]
            })),
            _ => contents.push(json!({
                "role": "user",
                "parts": [{ "text": message.content }],
            })),
        }
    }
    (system, contents)
}

pub fn parse_gemini_sse(
    reader: impl BufRead,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
) -> Result<Completion> {
    let mut completion = Completion::default();
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
        if let Some(usage) = value.get("usageMetadata") {
            if let Some(n) = usage.get("promptTokenCount").and_then(Value::as_u64) {
                completion.input_tokens = n;
            }
            if let Some(n) = usage.get("candidatesTokenCount").and_then(Value::as_u64) {
                completion.output_tokens = n;
            }
        }
        let Some(parts) = value
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for part in parts {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                completion.text.push_str(text);
                on_event(StreamEvent::TextDelta(text.to_owned()))?;
            }
            if let Some(call) = part.get("functionCall") {
                let name = call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned();
                let args = call.get("args").cloned().unwrap_or(json!({}));
                let tool = ToolCall {
                    id: format!("call_{name}"),
                    name,
                    arguments: args.to_string(),
                };
                on_event(StreamEvent::ToolCall(tool.clone()))?;
                completion.tool_calls.push(tool);
                completion.finish_reason = "tool_calls".to_owned();
            }
        }
        if let Some(reason) = value
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
        {
            if completion.finish_reason.is_empty() {
                completion.finish_reason = reason.to_ascii_lowercase();
            }
        }
    }
    if completion.finish_reason.is_empty() {
        completion.finish_reason = "stop".to_owned();
    }
    Ok(completion)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_text_and_function_call() {
        let body = "\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hi\"}]}}]}\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"list_dir\",\"args\":{\"path\":\".\"}}}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":2}}\n";
        let mut deltas = Vec::new();
        let completion = parse_gemini_sse(Cursor::new(body), &CancelToken::new(), &mut |event| {
            if let StreamEvent::TextDelta(text) = event {
                deltas.push(text);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(deltas, vec!["Hi".to_owned()]);
        assert_eq!(completion.tool_calls[0].name, "list_dir");
        assert_eq!(completion.input_tokens, 4);
    }
}
