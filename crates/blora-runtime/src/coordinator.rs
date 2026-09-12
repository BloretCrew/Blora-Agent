// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{
    AssistantDelta, AssistantMessageCompleted, EventEnvelope, KnownPayload, ModelRequested,
    ModelResponseCompleted, NewEvent, RunCancelRequested, RunCancelled, RunCompleted, RunCreated,
    RunFailed, RunStarted, ToolCompleted, ToolFailed, ToolOutput, ToolRequested, UsageRecorded,
    UserInput,
};
use blora_exec::LocalBackend;
use blora_model::{
    ChatMessage, CompletionRequest, MockProvider, OpenAiProvider, Provider, StreamEvent, ToolCall,
    ToolDeclaration, system_prompt,
};
use blora_policy::Policy;
use blora_session::SessionProjection;
use blora_storage::{CreateSession, SessionSummary, SqliteStore};
use blora_tools::ToolRegistry;
use blora_types::{BloraError, CancelToken, Mode, Result, RunId, SessionId, TurnId};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct MockRunOptions {
    pub model: String,
    pub chunks: Vec<String>,
}

impl Default for MockRunOptions {
    fn default() -> Self {
        Self {
            model: "mock".to_owned(),
            chunks: vec!["Hello".to_owned(), ", ".to_owned(), "world.".to_owned()],
        }
    }
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub model: String,
    pub mock: bool,
    pub auto_approve: bool,
    pub max_turns: u32,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            model: String::new(),
            mock: false,
            auto_approve: false,
            max_turns: 12,
        }
    }
}

pub struct Runtime {
    store: SqliteStore,
}

impl Runtime {
    #[must_use]
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }

    pub fn create_session(&self, spec: CreateSession) -> Result<SessionId> {
        self.store.create_session(spec)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        self.store.list_sessions()
    }

    pub fn show_session(&self, session_id: &SessionId) -> Result<SessionProjection> {
        self.store.load_projection(session_id)
    }

    pub fn replay_session(&self, session_id: &SessionId) -> Result<SessionProjection> {
        self.store.load_projection(session_id)
    }

    pub fn events(&self, session_id: &SessionId) -> Result<Vec<EventEnvelope>> {
        self.store.load_events(session_id)
    }

    pub fn run_mock(
        &self,
        session_id: &SessionId,
        prompt: &str,
        cancel: &CancelToken,
        options: &MockRunOptions,
    ) -> Result<RunId> {
        if prompt.trim().is_empty() {
            return Err(BloraError::Other("prompt must not be empty".to_owned()));
        }
        let run_id = RunId::generate();
        let turn_id = TurnId::generate();
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunCreated(RunCreated {
                mode: Mode::Code,
                model: Some(options.model.clone()),
            }),
        )?;
        if cancel.is_cancelled() {
            return self.cancel_run(session_id, &run_id, &turn_id);
        }
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunStarted(RunStarted {
                model: Some(options.model.clone()),
            }),
        )?;
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::UserInput(UserInput {
                text: prompt.to_owned(),
            }),
        )?;
        let mut assembled = String::new();
        for chunk in &options.chunks {
            if cancel.is_cancelled() {
                return self.cancel_run(session_id, &run_id, &turn_id);
            }
            assembled.push_str(chunk);
            self.emit(
                session_id,
                Some(&run_id),
                Some(&turn_id),
                KnownPayload::AssistantDelta(AssistantDelta {
                    text: chunk.clone(),
                }),
            )?;
        }
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::AssistantMessageCompleted(AssistantMessageCompleted { text: assembled }),
        )?;
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunCompleted(RunCompleted {
                summary: Some("mock completed".to_owned()),
            }),
        )?;
        Ok(run_id)
    }

    pub fn run(
        &self,
        session_id: &SessionId,
        prompt: &str,
        cancel: &CancelToken,
        options: &RunOptions,
    ) -> Result<RunId> {
        if prompt.trim().is_empty() {
            return Err(BloraError::Other("prompt must not be empty".to_owned()));
        }
        let projection = self.store.load_projection(session_id)?;
        let session = projection
            .session
            .as_ref()
            .ok_or_else(|| BloraError::SessionNotFound(session_id.to_string()))?;
        let workspace = session.workspace_path.clone();
        let mode = session.mode;
        let policy = Policy::new(&workspace, options.auto_approve)?;
        let backend = LocalBackend::new(policy);
        let provider: Box<dyn Provider> = if options.mock {
            Box::new(MockProvider::new())
        } else {
            match OpenAiProvider::from_env() {
                Ok(provider) => Box::new(provider),
                Err(_) => Box::new(MockProvider::new()),
            }
        };
        let model = if options.model.is_empty() {
            if options.mock || provider.name() == "mock" {
                "mock".to_owned()
            } else {
                std::env::var("BLORA_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_owned())
            }
        } else {
            options.model.clone()
        };

        let run_id = RunId::generate();
        let turn_id = TurnId::generate();
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunCreated(RunCreated {
                mode,
                model: Some(model.clone()),
            }),
        )?;
        if cancel.is_cancelled() {
            return self.cancel_run(session_id, &run_id, &turn_id);
        }
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunStarted(RunStarted {
                model: Some(model.clone()),
            }),
        )?;
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::UserInput(UserInput {
                text: prompt.to_owned(),
            }),
        )?;

        let tools = ToolRegistry::specs()
            .into_iter()
            .map(|spec| ToolDeclaration {
                name: spec.name.to_owned(),
                description: spec.description.to_owned(),
                parameters: spec.parameters,
            })
            .collect::<Vec<_>>();

        for turn in 0..options.max_turns {
            if cancel.is_cancelled() {
                return self.cancel_run(session_id, &run_id, &turn_id);
            }
            let events = self.store.load_events(session_id)?;
            let messages = compile_messages(&events, &workspace, mode.as_str())?;
            self.emit(
                session_id,
                Some(&run_id),
                Some(&turn_id),
                KnownPayload::ModelRequested(ModelRequested {
                    provider: provider.name().to_owned(),
                    model: model.clone(),
                }),
            )?;
            let mut streamed = String::new();
            let completion = match provider.complete(
                &CompletionRequest {
                    model: model.clone(),
                    messages,
                    tools: tools.clone(),
                },
                cancel,
                &mut |event| match event {
                    StreamEvent::TextDelta(text) => {
                        streamed.push_str(&text);
                        self.emit(
                            session_id,
                            Some(&run_id),
                            Some(&turn_id),
                            KnownPayload::AssistantDelta(AssistantDelta { text }),
                        )
                    }
                    StreamEvent::ToolCall(_) => Ok(()),
                },
            ) {
                Ok(completion) => completion,
                Err(BloraError::Cancelled) => {
                    return self.cancel_run(session_id, &run_id, &turn_id);
                }
                Err(err) => {
                    self.emit(
                        session_id,
                        Some(&run_id),
                        Some(&turn_id),
                        KnownPayload::RunFailed(RunFailed {
                            error: err.to_string(),
                        }),
                    )?;
                    return Err(err);
                }
            };

            let assistant_text = if completion.text.is_empty() {
                streamed.clone()
            } else {
                completion.text.clone()
            };
            if !assistant_text.is_empty() && streamed.is_empty() {
                self.emit(
                    session_id,
                    Some(&run_id),
                    Some(&turn_id),
                    KnownPayload::AssistantDelta(AssistantDelta {
                        text: assistant_text.clone(),
                    }),
                )?;
            }
            if !assistant_text.is_empty() {
                self.emit(
                    session_id,
                    Some(&run_id),
                    Some(&turn_id),
                    KnownPayload::AssistantMessageCompleted(AssistantMessageCompleted {
                        text: assistant_text,
                    }),
                )?;
            }

            self.emit(
                session_id,
                Some(&run_id),
                Some(&turn_id),
                KnownPayload::UsageRecorded(UsageRecorded {
                    input_tokens: completion.input_tokens,
                    output_tokens: completion.output_tokens,
                    cached_tokens: 0,
                }),
            )?;
            self.emit(
                session_id,
                Some(&run_id),
                Some(&turn_id),
                KnownPayload::ModelResponseCompleted(ModelResponseCompleted {
                    finish_reason: Some(completion.finish_reason.clone()),
                }),
            )?;

            if completion.tool_calls.is_empty() {
                self.emit(
                    session_id,
                    Some(&run_id),
                    Some(&turn_id),
                    KnownPayload::RunCompleted(RunCompleted {
                        summary: Some(format!("completed in {} model turns", turn + 1)),
                    }),
                )?;
                return Ok(run_id);
            }

            for call in completion.tool_calls {
                if cancel.is_cancelled() {
                    return self.cancel_run(session_id, &run_id, &turn_id);
                }
                self.dispatch_tool(session_id, &run_id, &turn_id, &backend, &call)?;
            }
        }

        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunFailed(RunFailed {
                error: "maximum model turns reached".to_owned(),
            }),
        )?;
        Err(BloraError::Other("maximum model turns reached".to_owned()))
    }

    fn dispatch_tool(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        backend: &LocalBackend,
        call: &ToolCall,
    ) -> Result<()> {
        let arguments: Value = serde_json::from_str(&call.arguments)
            .unwrap_or_else(|_| serde_json::json!({ "raw": call.arguments }));
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::ToolRequested(ToolRequested {
                tool: call.name.clone(),
                arguments: arguments.clone(),
                call_id: Some(call.id.clone()),
            }),
        )?;
        match ToolRegistry::execute(backend, &call.name, &arguments) {
            Ok(text) => {
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolOutput(ToolOutput {
                        text: text.clone(),
                        call_id: Some(call.id.clone()),
                    }),
                )?;
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolCompleted(ToolCompleted {
                        tool: call.name.clone(),
                        ok: true,
                    }),
                )?;
            }
            Err(BloraError::ApprovalRequired(summary)) => {
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolFailed(ToolFailed {
                        tool: call.name.clone(),
                        error: format!("approval required: {summary} (pass --yes to auto-approve)"),
                    }),
                )?;
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolOutput(ToolOutput {
                        text: format!(
                            "approval required for {summary}. Ask the user to rerun with --yes."
                        ),
                        call_id: Some(call.id.clone()),
                    }),
                )?;
            }
            Err(err) => {
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolFailed(ToolFailed {
                        tool: call.name.clone(),
                        error: err.to_string(),
                    }),
                )?;
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolOutput(ToolOutput {
                        text: format!("tool error: {err}"),
                        call_id: Some(call.id.clone()),
                    }),
                )?;
            }
        }
        Ok(())
    }

    fn cancel_run(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
    ) -> Result<RunId> {
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::RunCancelRequested(RunCancelRequested {
                reason: Some("user".to_owned()),
            }),
        )?;
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::RunCancelled(RunCancelled {
                reason: Some("user".to_owned()),
            }),
        )?;
        Err(BloraError::Cancelled)
    }

    fn emit(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        turn_id: Option<&TurnId>,
        payload: KnownPayload,
    ) -> Result<()> {
        let mut event = NewEvent::new(session_id.clone(), payload);
        if let Some(run_id) = run_id {
            event = event.with_run(run_id.clone());
        }
        if let Some(turn_id) = turn_id {
            event = event.with_turn(turn_id.clone());
        }
        self.store.append(event)?;
        Ok(())
    }
}

fn compile_messages(
    events: &[EventEnvelope],
    workspace: &str,
    mode: &str,
) -> Result<Vec<ChatMessage>> {
    let mut messages = vec![ChatMessage {
        role: "system".to_owned(),
        content: Some(system_prompt(workspace, mode)),
        tool_call_id: None,
        tool_calls: None,
    }];
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

    for event in events {
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
    use blora_session::TranscriptItem;
    use blora_storage::SqliteStore;

    #[test]
    fn mock_run_persists_transcript() {
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: Some("t".to_owned()),
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        runtime
            .run_mock(
                &session,
                "hello",
                &CancelToken::new(),
                &MockRunOptions::default(),
            )
            .unwrap();
        let projection = runtime.show_session(&session).unwrap();
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.status.as_str() == "completed")
        );
        assert!(projection.transcript.iter().any(
            |item| matches!(item, TranscriptItem::Assistant { text, .. } if text == "Hello, world.")
        ));
    }

    #[test]
    fn cancel_before_stream_is_idempotent() {
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = runtime
            .run_mock(&session, "hello", &cancel, &MockRunOptions::default())
            .unwrap_err();
        assert!(matches!(err, BloraError::Cancelled));
        let projection = runtime.show_session(&session).unwrap();
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.status.as_str() == "cancelled")
        );
        assert!(projection.runs.iter().all(|run| run.cancel_requested));
    }

    #[test]
    fn tool_loop_lists_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: dir.path().display().to_string(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        runtime
            .run(
                &session,
                "list files",
                &CancelToken::new(),
                &RunOptions {
                    mock: true,
                    auto_approve: true,
                    ..RunOptions::default()
                },
            )
            .unwrap();
        let projection = runtime.show_session(&session).unwrap();
        assert!(
            projection.transcript.iter().any(
                |item| matches!(item, TranscriptItem::Tool { name, .. } if name == "list_dir")
            )
        );
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.status.as_str() == "completed")
        );
    }
}
