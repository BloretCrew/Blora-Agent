// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use crate::{HarnessError, HarnessEvent, Outcome, Result};
use blora_types::HarnessKind;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub(crate) struct Parser {
    kind: HarnessKind,
    outcome: Outcome,
    terminal: bool,
    text_seen: bool,
    message_id: String,
    blocks: HashMap<String, String>,
    tools: HashSet<String>,
    finished_tools: HashSet<String>,
    streamed: HashSet<String>,
    usage: (u64, u64, u64),
    records: usize,
    stream_tools: HashMap<u64, (String, String, String)>,
}

impl Parser {
    pub(crate) fn new(kind: HarnessKind) -> Self {
        Self {
            kind,
            outcome: Outcome::default(),
            terminal: false,
            text_seen: false,
            message_id: String::new(),
            blocks: HashMap::new(),
            tools: HashSet::new(),
            finished_tools: HashSet::new(),
            streamed: HashSet::new(),
            usage: (0, 0, 0),
            records: 0,
            stream_tools: HashMap::new(),
        }
    }

    pub(crate) fn line(&mut self, bytes: &[u8], emit: &mut impl FnMut(HarnessEvent)) -> Result<()> {
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(());
        }
        self.records += 1;
        if self.records > 100_000 {
            return Err(HarnessError::Protocol("too many protocol records".into()));
        }
        let v: Value = serde_json::from_slice(bytes)
            .map_err(|_| HarnessError::Protocol("invalid JSON on stdout".into()))?;
        if !v.is_object() {
            return Err(HarnessError::Protocol("expected an event object".into()));
        }
        if self.terminal {
            return Err(HarnessError::Protocol("event after terminal record".into()));
        }
        match self.kind {
            HarnessKind::ClaudeCode => self.claude(&v, emit),
            HarnessKind::Codex => self.codex(&v, emit),
            HarnessKind::GrokBuild => self.grok(&v, emit),
            HarnessKind::Blora => unreachable!(),
        }
    }

    pub(crate) fn finish(self) -> Result<Outcome> {
        if !self.terminal {
            return Err(HarnessError::Protocol(
                "missing terminal event; update CLI or check authentication".into(),
            ));
        }
        Ok(self.outcome)
    }

    fn session(
        &mut self,
        id: &str,
        model: Option<&str>,
        emit: &mut impl FnMut(HarnessEvent),
    ) -> Result<()> {
        if id.is_empty() {
            return Ok(());
        }
        if self
            .outcome
            .session_id
            .as_deref()
            .is_some_and(|old| old != id)
        {
            return Err(HarnessError::Protocol(
                "session ID changed during run".into(),
            ));
        }
        self.outcome.session_id = Some(id.into());
        if let Some(model) = model {
            self.outcome.model = Some(model.into());
        }
        emit(HarnessEvent::Session {
            session_id: id.into(),
            model: self.outcome.model.clone(),
        });
        Ok(())
    }

    fn text(&mut self, text: &str, reasoning: bool, emit: &mut impl FnMut(HarnessEvent)) {
        if text.is_empty() {
            return;
        }
        if reasoning {
            emit(HarnessEvent::Reasoning { text: text.into() });
        } else {
            self.text_seen = true;
            emit(HarnessEvent::Text { text: text.into() });
        }
    }

    fn start(
        &mut self,
        id: &str,
        name: &str,
        input: Value,
        emit: &mut impl FnMut(HarnessEvent),
    ) -> Result<()> {
        if id.is_empty() {
            return Err(HarnessError::Protocol("tool record missing call ID".into()));
        }
        if self.tools.insert(id.into()) {
            emit(HarnessEvent::ToolStarted {
                call_id: id.into(),
                name: name.into(),
                input,
            });
        }
        Ok(())
    }

    fn end(
        &mut self,
        id: &str,
        output: &Value,
        error: bool,
        emit: &mut impl FnMut(HarnessEvent),
    ) -> Result<()> {
        if id.is_empty() {
            return Err(HarnessError::Protocol("tool result missing call ID".into()));
        }
        if self.finished_tools.insert(id.into()) {
            emit(HarnessEvent::ToolFinished {
                call_id: id.into(),
                output: output
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| output.to_string()),
                is_error: error,
            });
        }
        Ok(())
    }

    fn usage(&mut self, v: &Value, cumulative: bool, emit: &mut impl FnMut(HarnessEvent)) {
        let next = (
            number(v, "input_tokens"),
            number(v, "output_tokens"),
            number(v, "cache_read_input_tokens").max(number(v, "cached_input_tokens")),
        );
        if cumulative {
            self.usage = next;
        } else {
            self.usage = (
                self.usage.0.saturating_add(next.0),
                self.usage.1.saturating_add(next.1),
                self.usage.2.saturating_add(next.2),
            );
        }
        emit(HarnessEvent::Usage {
            input_tokens: self.usage.0,
            output_tokens: self.usage.1,
            cached_input_tokens: self.usage.2,
        });
    }

    fn claude(&mut self, v: &Value, emit: &mut impl FnMut(HarnessEvent)) -> Result<()> {
        // Child-agent text is not the parent reply, but tool lifecycles remain visible.
        let child = v.get("parent_tool_use_id").is_some_and(|p| !p.is_null());
        match string(v, "type") {
            "system" => {
                self.session(
                    string(v, "session_id"),
                    v.get("model").and_then(Value::as_str),
                    emit,
                )?;
            }
            "stream_event" if !child => {
                let e = &v["event"];
                match string(e, "type") {
                    "message_start" => {
                        self.message_id = string(&e["message"], "id").into();
                        self.stream_tools.clear();
                    }
                    "content_block_start" => {
                        let b = &e["content_block"];
                        let key = format!("{}:{}", self.message_id, number(e, "index"));
                        if string(b, "type") == "tool_use" {
                            self.stream_tools.insert(
                                number(e, "index"),
                                (
                                    string(b, "id").into(),
                                    string(b, "name").into(),
                                    String::new(),
                                ),
                            );
                        }
                        if string(b, "type") == "text" || string(b, "type") == "thinking" {
                            self.streamed.insert(key);
                            self.text(
                                string(
                                    b,
                                    if string(b, "type") == "thinking" {
                                        "thinking"
                                    } else {
                                        "text"
                                    },
                                ),
                                string(b, "type") == "thinking",
                                emit,
                            );
                        }
                    }
                    "content_block_delta" => {
                        let d = &e["delta"];
                        let key = format!("{}:{}", self.message_id, number(e, "index"));
                        match string(d, "type") {
                            "text_delta" => {
                                self.streamed.insert(key);
                                self.text(string(d, "text"), false, emit);
                            }
                            "thinking_delta" => {
                                self.streamed.insert(key);
                                self.text(string(d, "thinking"), true, emit);
                            }
                            "input_json_delta" => {
                                if let Some((_, _, input)) =
                                    self.stream_tools.get_mut(&number(e, "index"))
                                {
                                    input.push_str(string(d, "partial_json"));
                                    if input.len() > 1024 * 1024 {
                                        return Err(HarnessError::Protocol(
                                            "tool input exceeded 1 MiB".into(),
                                        ));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    "content_block_stop" => {
                        if let Some((id, name, input)) =
                            self.stream_tools.remove(&number(e, "index"))
                        {
                            let input = if input.is_empty() {
                                json!({})
                            } else {
                                serde_json::from_str(&input).map_err(|_| {
                                    HarnessError::Protocol("invalid streamed tool input".into())
                                })?
                            };
                            self.start(&id, &name, input, emit)?;
                        }
                    }
                    _ => {}
                }
            }
            "assistant" | "user" => {
                let message = &v["message"];
                let id = string(message, "id");
                if let Some(blocks) = message["content"].as_array() {
                    for (index, b) in blocks.iter().enumerate() {
                        match string(b, "type") {
                            "text" | "thinking" if !child => {
                                let key = format!("{id}:{index}");
                                if self.streamed.insert(key) {
                                    self.text(
                                        string(
                                            b,
                                            if string(b, "type") == "thinking" {
                                                "thinking"
                                            } else {
                                                "text"
                                            },
                                        ),
                                        string(b, "type") == "thinking",
                                        emit,
                                    );
                                }
                            }
                            "tool_use" => self.start(
                                string(b, "id"),
                                string(b, "name"),
                                b["input"].clone(),
                                emit,
                            )?,
                            "tool_result" => self.end(
                                string(b, "tool_use_id"),
                                &b["content"],
                                b["is_error"].as_bool().unwrap_or(false),
                                emit,
                            )?,
                            _ => {}
                        }
                    }
                }
            }
            "result" => {
                if v["is_error"].as_bool() == Some(true) || string(v, "subtype") != "success" {
                    return Err(HarnessError::Process("Claude reported unsuccessful result; check external configuration and permissions".into()));
                }
                self.session(string(v, "session_id"), None, emit)?;
                if !self.text_seen {
                    self.text(string(v, "result"), false, emit);
                }
                if v["usage"].is_object() {
                    self.usage(&v["usage"], true, emit);
                }
                if v["permission_denials"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
                {
                    emit(HarnessEvent::Warning { message: "External CLI denied tool permissions; interactive approval is unavailable".into() });
                }
                self.terminal = true;
            }
            "error" => {
                return Err(HarnessError::Process(
                    "Claude protocol reported an error".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn codex(&mut self, v: &Value, emit: &mut impl FnMut(HarnessEvent)) -> Result<()> {
        match string(v, "type") {
            "thread.started" => self.session(
                string(v, "thread_id"),
                v.get("model").and_then(Value::as_str),
                emit,
            )?,
            "item.started" | "item.updated" | "item.completed" => {
                let item = &v["item"];
                let id = string(item, "id");
                match string(item, "type") {
                    "agent_message" | "reasoning" => {
                        let text = string(item, "text");
                        let old = self.blocks.entry(id.into()).or_default();
                        if !text.starts_with(old.as_str()) {
                            return Err(HarnessError::Protocol(
                                "non-monotonic Codex message snapshot".into(),
                            ));
                        }
                        let suffix = text[old.len()..].to_owned();
                        *old = text.into();
                        self.text(&suffix, string(item, "type") == "reasoning", emit);
                    }
                    "command_execution" | "mcp_tool_call" | "file_change" | "web_search" => {
                        self.start(id, string(item, "type"), item.clone(), emit)?;
                        if string(v, "type") == "item.completed" {
                            self.end(
                                id,
                                item.get("aggregated_output").unwrap_or(item),
                                string(item, "status") == "failed"
                                    || item["exit_code"].as_i64().is_some_and(|c| c != 0),
                                emit,
                            )?;
                        }
                    }
                    "error" => return Err(HarnessError::Process("Codex item failed".into())),
                    _ => {}
                }
            }
            "turn.completed" => {
                self.usage(&v["usage"], true, emit);
                self.terminal = true;
            }
            "turn.failed" | "error" => {
                return Err(HarnessError::Process(
                    "Codex reported an error; check external authentication/configuration".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn grok(&mut self, v: &Value, emit: &mut impl FnMut(HarnessEvent)) -> Result<()> {
        match string(v, "type") {
            "text" => self.text(string(v, "data"), false, emit),
            "thought" => self.text(string(v, "data"), true, emit),
            "tool_call" => self.start(
                string(v, "toolCallId"),
                v.get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or(string(v, "title")),
                v.get("rawInput").cloned().unwrap_or(json!({})),
                emit,
            )?,
            "tool_call_update" => {
                if matches!(string(v, "status"), "completed" | "failed") {
                    self.end(
                        string(v, "toolCallId"),
                        v.get("rawOutput").unwrap_or(&v["content"]),
                        string(v, "status") == "failed",
                        emit,
                    )?;
                }
            }
            "usage" => self.usage(&v["usage"], false, emit),
            "end" => {
                match string(v, "stopReason") {
                    "cancelled" => return Err(HarnessError::Cancelled),
                    "end_turn" | "max_tokens" | "max_turn_requests" | "refusal" => {}
                    _ => {
                        return Err(HarnessError::Protocol(
                            "unrecognized Grok terminal stopReason".into(),
                        ));
                    }
                }
                self.session(
                    string(v, "sessionId"),
                    v.get("model").and_then(Value::as_str),
                    emit,
                )?;
                if v["usage"].is_object() {
                    self.usage(&v["usage"], true, emit);
                }
                self.terminal = true;
            }
            "error" => {
                return Err(HarnessError::Process(
                    "Grok reported an error; check external authentication/configuration".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn number(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}
