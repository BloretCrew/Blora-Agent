// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use blora_events::{
    AssistantDelta, AssistantMessageCompleted, AssistantReasoning, ExternalSessionBound,
    HarnessChanged, KnownPayload, ModelRequested, RunCompleted, RunCreated, RunFailed, RunStarted,
    ToolCompleted, ToolOutput, ToolRequested, ToolStarted, UsageRecorded,
};
use blora_harness::{HarnessError, HarnessEvent, RunRequest};
use blora_types::{BloraError, CancelToken, HarnessKind, Mode, Result, RunId, SessionId, TurnId};

use crate::{RunOptions, Runtime};

impl Runtime {
    /// Persist the foreground kernel. Native background and Web runs are unaffected.
    pub fn set_session_harness(&self, session_id: &SessionId, harness: HarnessKind) -> Result<()> {
        let active = self
            .foreground_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.contains(session_id) {
            return Err(BloraError::Other("运行期间不能切换执行内核".to_owned()));
        }
        let projection = self.show_session(session_id)?;
        if projection.runs.iter().any(|run| !run.status.is_terminal()) {
            return Err(BloraError::Other("运行期间不能切换执行内核".to_owned()));
        }
        if projection.harness == harness {
            return Ok(());
        }
        if harness != HarnessKind::Blora && blora_harness::detect(harness).is_none() {
            return Err(BloraError::Other(format!(
                "{} 未安装",
                harness.display_name()
            )));
        }
        self.emit(
            session_id,
            None,
            None,
            KnownPayload::HarnessChanged(HarnessChanged { harness }),
        )
    }

    /// TUI foreground entry; external kernels never resolve native providers or models.
    pub fn run_foreground(
        &self,
        session_id: &SessionId,
        prompt: &str,
        cancel: &CancelToken,
        options: &RunOptions,
    ) -> Result<RunId> {
        let mut active = self
            .foreground_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !active.insert(session_id.clone()) {
            return Err(BloraError::Other("当前会话已有前台请求".to_owned()));
        }
        drop(active);
        let _guard = ForegroundGuard {
            runtime: self,
            session_id,
        };
        let projection = self.show_session(session_id)?;
        let harness = projection.harness;
        if harness == HarnessKind::Blora {
            return self.run(session_id, prompt, cancel, options);
        }
        let first = self.run_external(session_id, prompt, cancel, options, harness)?;
        self.drain_external_queue(session_id, cancel, |message| {
            self.run_external(session_id, message, cancel, options, harness)
        })?;
        Ok(first)
    }

    fn drain_external_queue(
        &self,
        session_id: &SessionId,
        cancel: &CancelToken,
        mut run: impl FnMut(&str) -> Result<RunId>,
    ) -> Result<()> {
        while !cancel.is_cancelled() {
            let queued = self.store.take_steers(session_id)?;
            if queued.is_empty() {
                break;
            }
            // Steers are deferred turns, not live control messages to the external CLI.
            for (index, message) in queued.iter().enumerate() {
                if cancel.is_cancelled() {
                    self.restore_external_queue(session_id, &queued[index..])?;
                    return Err(BloraError::Cancelled);
                }
                let before = self.last_sequence(session_id)?;
                let result = run(message);
                if result.is_err() || cancel.is_cancelled() {
                    // A cancellation before user.input must not consume that queued input.
                    let recorded = self
                        .events(session_id)?
                        .iter()
                        .any(|event| event.sequence > before && event.event_type == "user.input");
                    let remaining = if recorded { index + 1 } else { index };
                    self.restore_external_queue(session_id, &queued[remaining..])?;
                    return match result {
                        Err(error) => Err(error),
                        Ok(_) => Err(BloraError::Cancelled),
                    };
                }
            }
        }
        Ok(())
    }

    fn restore_external_queue(&self, session_id: &SessionId, messages: &[String]) -> Result<()> {
        for message in messages {
            self.store.queue_steer(session_id, message)?;
        }
        Ok(())
    }

    fn run_external(
        &self,
        session_id: &SessionId,
        prompt: &str,
        cancel: &CancelToken,
        options: &RunOptions,
        harness: HarnessKind,
    ) -> Result<RunId> {
        if prompt.len() > 32 * 1024 {
            return Err(BloraError::Other(
                "外部 CLI 单次输入超过 32 KiB；请缩短输入".to_owned(),
            ));
        }
        if prompt.trim().is_empty() {
            return Err(BloraError::Other("prompt must not be empty".to_owned()));
        }
        let references = self.session_reference_context(session_id, prompt)?;
        let mut provider_prompt = prompt.to_owned();
        if let Some(references) = &references {
            provider_prompt.push_str(blora_context::SESSION_REFERENCE_HEADER);
            provider_prompt.push_str(&references.to_string());
        }
        let projection = self.show_session(session_id)?;
        let session = projection
            .session
            .as_ref()
            .ok_or_else(|| BloraError::SessionNotFound(session_id.to_string()))?;
        if projection.runs.iter().any(|run| !run.status.is_terminal()) {
            return Err(BloraError::Other("当前会话已有运行中的请求".to_owned()));
        }
        if session.mode == Mode::Imagine {
            return Err(BloraError::Other(
                "Imagine 仅支持 Blora 原生内核；请先切回 Blora".to_owned(),
            ));
        }
        if options.worktree
            || std::env::var("BLORA_WORKTREE")
                .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        {
            return Err(BloraError::Policy(
                "外部内核暂不支持 Blora worktree 隔离".to_owned(),
            ));
        }
        let root = std::fs::canonicalize(&session.workspace_path)
            .map_err(|error| BloraError::Other(error.to_string()))?;
        let cwd = std::fs::canonicalize(root.join(projection.shell_cwd.as_deref().unwrap_or(".")))
            .map_err(|error| BloraError::Other(error.to_string()))?;
        if !cwd.starts_with(&root) {
            return Err(BloraError::Policy(
                "外部工作目录超出会话项目边界".to_owned(),
            ));
        }
        let binding = projection.external_sessions.iter().find(|binding| {
            binding.harness == harness
                && PathBuf::from(&binding.cwd) == cwd
                && binding.synced_sequence <= projection.last_sequence
        });
        let request = RunRequest {
            kind: harness,
            cwd: cwd.clone(),
            prompt: context_prompt(
                &self.events(session_id)?,
                binding.map_or(0, |b| b.synced_sequence),
                &provider_prompt,
            ),
            resume_session_id: binding.map(|binding| binding.external_session_id.clone()),
            permission: options.permission_mode(),
        };
        if self.session_needs_title(session_id, &projection)? {
            let title_prompt = projection
                .transcript
                .iter()
                .find_map(|item| match item {
                    blora_session::TranscriptItem::User { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .unwrap_or(prompt);
            let title = crate::coordinator::derive_session_title(title_prompt);
            self.set_session_title(
                session_id,
                if title.is_empty() {
                    harness.display_name().to_owned()
                } else {
                    title
                },
                true,
            )?;
        }
        let run_id = RunId::generate();
        let turn_id = TurnId::generate();
        let emit = |payload| self.emit(session_id, Some(&run_id), Some(&turn_id), payload);
        emit(KnownPayload::RunCreated(RunCreated {
            mode: session.mode,
            model: None,
            permission_mode: Some(options.permission_mode().as_str().to_owned()),
        }))?;
        if cancel.is_cancelled() {
            return self.cancel_run(session_id, &run_id, &turn_id);
        }
        emit(KnownPayload::RunStarted(RunStarted { model: None }))?;
        self.emit_referenced_input(session_id, &run_id, &turn_id, prompt.to_owned(), references)?;
        let mut text = String::new();
        let mut tools = HashMap::new();
        let mut warnings = Vec::new();
        let mut seen_warnings = HashSet::new();
        let mut usage = (0_u64, 0_u64, 0_u64);
        let mut callback_error = None;
        let process_cancel = cancel.clone();
        let outcome = blora_harness::run(&request, &process_cancel, |event| {
            if callback_error.is_some() {
                return;
            }
            let result = match event {
                HarnessEvent::Text { text: delta } => {
                    text.push_str(&delta);
                    emit(KnownPayload::AssistantDelta(AssistantDelta { text: delta }))
                }
                HarnessEvent::Reasoning { text } => {
                    emit(KnownPayload::AssistantReasoning(AssistantReasoning {
                        text,
                    }))
                }
                HarnessEvent::ToolStarted {
                    call_id,
                    name,
                    input,
                } => {
                    tools.insert(call_id.clone(), name.clone());
                    emit(KnownPayload::ToolRequested(ToolRequested {
                        tool: name.clone(),
                        arguments: input,
                        call_id: Some(call_id.clone()),
                    }))
                    .and_then(|()| {
                        self.emit_external_tool(
                            session_id,
                            &run_id,
                            &turn_id,
                            &call_id,
                            KnownPayload::ToolStarted(ToolStarted { tool: name }),
                        )
                    })
                }
                HarnessEvent::ToolFinished {
                    call_id,
                    output,
                    is_error,
                } => {
                    let name = tools
                        .remove(&call_id)
                        .unwrap_or_else(|| "external tool".to_owned());
                    emit(KnownPayload::ToolOutput(ToolOutput {
                        text: output,
                        call_id: Some(call_id.clone()),
                        truncated: false,
                    }))
                    .and_then(|()| {
                        self.emit_external_tool(
                            session_id,
                            &run_id,
                            &turn_id,
                            &call_id,
                            KnownPayload::ToolCompleted(ToolCompleted {
                                tool: name,
                                ok: !is_error,
                            }),
                        )
                    })
                }
                HarnessEvent::Usage {
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                } => {
                    let next = (input_tokens, output_tokens, cached_input_tokens);
                    let result = emit(KnownPayload::UsageRecorded(UsageRecorded {
                        input_tokens: next.0.saturating_sub(usage.0),
                        output_tokens: next.1.saturating_sub(usage.1),
                        cached_tokens: next.2.saturating_sub(usage.2),
                    }));
                    usage = (
                        usage.0.max(next.0),
                        usage.1.max(next.1),
                        usage.2.max(next.2),
                    );
                    result
                }
                HarnessEvent::Session { model, .. } => match model {
                    Some(model) => emit(KnownPayload::ModelRequested(ModelRequested {
                        provider: harness.as_str().to_owned(),
                        model,
                    })),
                    None => Ok(()),
                },
                HarnessEvent::Warning { message } => {
                    collect_warning(&mut seen_warnings, &mut warnings, message);
                    Ok(())
                }
            };
            if let Err(error) = result {
                callback_error = Some(error);
                process_cancel.cancel();
            }
        });
        for (call_id, name) in tools {
            self.emit_external_tool(
                session_id,
                &run_id,
                &turn_id,
                &call_id,
                KnownPayload::ToolCompleted(ToolCompleted {
                    tool: name,
                    ok: false,
                }),
            )?;
        }
        // Commit partial output even when the process failed or was cancelled.
        if !text.is_empty() {
            emit(KnownPayload::AssistantMessageCompleted(
                AssistantMessageCompleted { text },
            ))?;
        }
        for message in warnings {
            self.emit_external_warning(session_id, &run_id, &turn_id, harness, &message)?;
        }
        if let Some(error) = callback_error {
            self.invalidate_external(session_id, harness, &cwd)?;
            emit(KnownPayload::RunFailed(RunFailed {
                error: error.to_string(),
            }))?;
            return Err(error);
        }
        match outcome {
            Ok(outcome) => {
                emit(KnownPayload::RunCompleted(RunCompleted {
                    summary: Some(format!("{} completed", harness.display_name())),
                }))?;
                if let Some(external_session_id) = outcome.session_id {
                    let synced_sequence = self.show_session(session_id)?.last_sequence;
                    self.emit(
                        session_id,
                        None,
                        None,
                        KnownPayload::ExternalSessionBound(ExternalSessionBound {
                            harness,
                            external_session_id,
                            cwd: cwd.to_string_lossy().into_owned(),
                            synced_sequence,
                        }),
                    )?;
                } else {
                    self.invalidate_external(session_id, harness, &cwd)?;
                }
                Ok(run_id)
            }
            Err(error) => {
                self.invalidate_external(session_id, harness, &cwd)?;
                if matches!(error, HarnessError::Cancelled) {
                    self.cancel_run(session_id, &run_id, &turn_id)
                } else {
                    emit(KnownPayload::RunFailed(RunFailed {
                        error: error.to_string(),
                    }))?;
                    Err(BloraError::Other(error.to_string()))
                }
            }
        }
    }

    fn emit_external_warning(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        harness: HarnessKind,
        message: &str,
    ) -> Result<()> {
        let payload = KnownPayload::AssistantMessageCompleted(AssistantMessageCompleted {
            text: format!("[{} 状态提示] {message}", harness.display_name()),
        });
        let mut event = blora_events::NewEvent::new(session_id.clone(), payload)
            .with_run(run_id.clone())
            .with_turn(turn_id.clone());
        event.actor = blora_types::Actor::System;
        event
            .metadata
            .insert("external_warning".into(), serde_json::Value::Bool(true));
        let stored = self.store.append(event)?;
        self.notify(&stored);
        Ok(())
    }

    fn emit_external_tool(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        call_id: &str,
        payload: KnownPayload,
    ) -> Result<()> {
        let mut event = blora_events::NewEvent::new(session_id.clone(), payload)
            .with_run(run_id.clone())
            .with_turn(turn_id.clone());
        event.metadata.insert(
            "external_call_id".into(),
            serde_json::Value::String(call_id.to_owned()),
        );
        let stored = self.store.append(event)?;
        self.notify(&stored);
        Ok(())
    }

    fn invalidate_external(
        &self,
        session_id: &SessionId,
        harness: HarnessKind,
        cwd: &std::path::Path,
    ) -> Result<()> {
        self.emit(
            session_id,
            None,
            None,
            KnownPayload::ExternalSessionBound(ExternalSessionBound {
                harness,
                external_session_id: String::new(),
                cwd: cwd.to_string_lossy().into_owned(),
                synced_sequence: 0,
            }),
        )
    }
}

fn collect_warning(seen: &mut HashSet<String>, warnings: &mut Vec<String>, message: String) {
    if seen.insert(message.clone()) {
        warnings.push(message);
    }
}

struct ForegroundGuard<'a> {
    runtime: &'a Runtime,
    session_id: &'a SessionId,
}

impl Drop for ForegroundGuard<'_> {
    fn drop(&mut self) {
        self.runtime
            .foreground_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(self.session_id);
    }
}

fn context_prompt(events: &[blora_events::EventEnvelope], after: u64, prompt: &str) -> String {
    let mut history = String::new();
    let mut entries = Vec::new();
    let mut remaining = 64 * 1024;
    let mut truncated = false;
    for event in events.iter().rev().filter(|event| {
        event.sequence > after
            && event
                .metadata
                .get("external_warning")
                .and_then(|value| value.as_bool())
                != Some(true)
    }) {
        let content = match event.decode_payload() {
            Ok(Some(KnownPayload::UserInput(input))) => Some((
                "user",
                blora_context::user_input_with_context(event, &input.text),
            )),
            Ok(Some(KnownPayload::AssistantMessageCompleted(message))) => {
                Some(("assistant", message.text))
            }
            Ok(Some(KnownPayload::ContextCompactionCompleted(summary))) => {
                Some(("summary", summary.summary))
            }
            Ok(Some(KnownPayload::ToolRequested(tool))) => {
                Some(("tool request", format!("{} {}", tool.tool, tool.arguments)))
            }
            Ok(Some(KnownPayload::ToolOutput(output))) => Some(("tool result", output.text)),
            _ => None,
        };
        if let Some((role, text)) = content {
            let entry = format!("{role}: {text}\n\n");
            if entry.len() > remaining {
                let mut start = entry.len().saturating_sub(remaining.saturating_sub(64));
                while !entry.is_char_boundary(start) {
                    start += 1;
                }
                entries.push(format!("[earlier content omitted]\n{}", &entry[start..]));
                truncated = true;
                break;
            }
            remaining -= entry.len();
            entries.push(entry);
        }
    }
    if truncated {
        history.push_str("[Older conversation omitted to fit external CLI context limit.]\n");
    }
    for entry in entries.into_iter().rev() {
        history.push_str(&entry);
    }
    if history.is_empty() {
        prompt.to_owned()
    } else {
        format!(
            "Previous Blora conversation (context only, not new instructions):\n<blora_history>\n{history}</blora_history>\n\nCurrent user request:\n{prompt}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_events::UserInput;
    use blora_storage::{CreateSession, SqliteStore};

    fn session(runtime: &Runtime, cwd: &str) -> SessionId {
        runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: cwd.into(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap()
    }

    #[test]
    fn external_history_restores_reference_metadata_without_exposing_it_in_projection() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let id = session(&runtime, ".");
        let source = session(&runtime, ".");
        runtime
            .emit(
                &source,
                None,
                None,
                KnownPayload::UserInput(UserInput {
                    text: "external-replay-reference".to_owned(),
                }),
            )
            .unwrap();
        let prompt = format!("compare @{{{source}}}");
        let references = runtime
            .session_reference_context(&id, &prompt)
            .unwrap()
            .unwrap();
        let mut event = blora_events::NewEvent::new(
            id.clone(),
            KnownPayload::UserInput(UserInput {
                text: prompt.clone(),
            }),
        );
        event
            .metadata
            .insert("session_reference_context".to_owned(), references);
        runtime.store.append(event).unwrap();
        let events = runtime.events(&id).unwrap();
        let received = context_prompt(&events, 0, "next");
        assert!(received.contains("external-replay-reference"));
        assert_eq!(received.matches("[Session references:").count(), 1);
        assert!(
            runtime
                .replay_session(&id)
                .unwrap()
                .transcript
                .iter()
                .any(|item| matches!(item,
                    blora_session::TranscriptItem::User { text, .. } if text == &prompt
                ))
        );
    }

    #[test]
    fn harness_replay_context_and_fork_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let id = session(&runtime, dir.path().to_str().unwrap());
        assert_eq!(
            runtime.show_session(&id).unwrap().harness,
            HarnessKind::Blora
        );
        runtime
            .emit(
                &id,
                None,
                None,
                KnownPayload::HarnessChanged(HarnessChanged {
                    harness: HarnessKind::Codex,
                }),
            )
            .unwrap();
        runtime
            .emit(
                &id,
                None,
                None,
                KnownPayload::ExternalSessionBound(ExternalSessionBound {
                    harness: HarnessKind::Codex,
                    external_session_id: "thread-1".into(),
                    cwd: dir.path().to_str().unwrap().into(),
                    synced_sequence: 1,
                }),
            )
            .unwrap();
        let mut future = runtime.events(&id).unwrap().last().unwrap().clone();
        future.sequence += 1;
        future.event_type = "future.unknown".into();
        future.payload = serde_json::json!({});
        let mut projection = runtime.replay_session(&id).unwrap();
        projection.apply(&future).unwrap();
        assert_eq!(projection.harness, HarnessKind::Codex);
        assert_eq!(projection.external_sessions.len(), 1);
        let fork = runtime.fork_session(&id).unwrap();
        assert!(
            runtime
                .show_session(&fork)
                .unwrap()
                .external_sessions
                .is_empty()
        );
        runtime
            .emit(
                &id,
                None,
                None,
                KnownPayload::SessionCwdChanged(blora_events::SessionCwdChanged {
                    cwd: "child".into(),
                }),
            )
            .unwrap();
        assert!(
            runtime
                .show_session(&id)
                .unwrap()
                .external_sessions
                .is_empty()
        );
    }

    #[test]
    fn external_model_does_not_overwrite_native_routing() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let id = session(&runtime, "/tmp");
        runtime
            .set_session_routing(&id, "mock", "native-model")
            .unwrap();
        runtime
            .emit(
                &id,
                None,
                None,
                KnownPayload::ModelRequested(ModelRequested {
                    provider: "codex".into(),
                    model: "external-model".into(),
                }),
            )
            .unwrap();
        let projection = runtime.show_session(&id).unwrap();
        assert_eq!(projection.model.as_deref(), Some("native-model"));
        assert_eq!(projection.provider.as_deref(), Some("mock"));
        assert_eq!(projection.external_model.as_deref(), Some("external-model"));
    }

    #[test]
    fn context_cursor_only_sends_missing_history() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let id = session(&runtime, "/tmp");
        runtime
            .run_mock(
                &id,
                "old-user",
                &CancelToken::new(),
                &crate::MockRunOptions::default(),
            )
            .unwrap();
        let cursor = runtime.show_session(&id).unwrap().last_sequence;
        let all = context_prompt(&runtime.events(&id).unwrap(), 0, "new-user");
        assert!(all.contains("old-user"));
        assert_eq!(all.matches("new-user").count(), 1);
        assert_eq!(
            context_prompt(&runtime.events(&id).unwrap(), cursor, "new-user"),
            "new-user"
        );
    }

    #[test]
    fn foreground_native_and_external_imagine_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let id = session(&runtime, dir.path().to_str().unwrap());
        runtime
            .run_foreground(
                &id,
                "hello",
                &CancelToken::new(),
                &RunOptions {
                    mock: true,
                    ..Default::default()
                },
            )
            .unwrap();
        runtime.set_session_mode(&id, Mode::Imagine).unwrap();
        runtime
            .emit(
                &id,
                None,
                None,
                KnownPayload::HarnessChanged(HarnessChanged {
                    harness: HarnessKind::Codex,
                }),
            )
            .unwrap();
        assert!(
            runtime
                .run_foreground(&id, "picture", &CancelToken::new(), &RunOptions::default())
                .unwrap_err()
                .to_string()
                .contains("Imagine")
        );
        assert!(runtime.foreground_sessions.lock().unwrap().is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn fake_cli_lifecycle_resume_queue_failure_and_cancel() {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(root) = std::env::var("BLORA_RUNTIME_FAKE_ROOT") {
            let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
            let id = session(&runtime, &root);
            runtime
                .set_session_harness(&id, HarnessKind::Codex)
                .unwrap();
            let options = RunOptions {
                provider: "invalid-native-provider".into(),
                model: "do-not-send".into(),
                ..Default::default()
            };
            runtime.queue_steer(&id, "queued-message").unwrap();
            runtime
                .run_foreground(&id, "first-message", &CancelToken::new(), &options)
                .unwrap();
            let projection = runtime.replay_session(&id).unwrap();
            assert_eq!(projection.runs.len(), 2);
            assert_eq!(
                projection.external_sessions[0].external_session_id,
                "thread-1"
            );
            assert_eq!(projection.input_tokens, 20);
            assert_eq!(projection.output_tokens, 8);
            assert!(projection.transcript.iter().any(|item| matches!(item, blora_session::TranscriptItem::Tool { call_id: Some(call), status, .. } if call == "tool-1" && status == "completed")));
            assert_eq!(
                projection.session.unwrap().title.as_deref(),
                Some("first-message")
            );
            let invocations = std::fs::read_to_string(format!("{root}/invocations")).unwrap();
            assert!(invocations.contains("resume"));
            assert!(!invocations.contains("do-not-send"));
            assert_eq!(invocations.matches("first-message").count(), 1);
            let source = session(&runtime, &root);
            runtime
                .emit(
                    &source,
                    None,
                    None,
                    KnownPayload::UserInput(UserInput {
                        text: "external-reference-content".to_owned(),
                    }),
                )
                .unwrap();
            let mention = format!("compare @{{{source}}}");
            runtime.queue_steer(&id, &mention).unwrap();
            runtime
                .run_foreground(&id, "before-reference", &CancelToken::new(), &options)
                .unwrap();
            let invocations = std::fs::read_to_string(format!("{root}/invocations")).unwrap();
            assert_eq!(invocations.matches("external-reference-content").count(), 1);
            assert_eq!(invocations.matches("[Session references:").count(), 1);
            let stored = runtime
                .events(&id)
                .unwrap()
                .into_iter()
                .filter(|event| event.event_type == "user.input")
                .last()
                .unwrap();
            let text = stored.payload["text"].as_str().unwrap();
            assert_eq!(text, mention);
            assert!(!text.contains("[Session references:"));
            assert!(
                stored.metadata["session_reference_context"]
                    .to_string()
                    .contains("external-reference-content")
            );
            runtime
                .set_session_harness(&id, HarnessKind::Blora)
                .unwrap();
            runtime
                .run_mock(
                    &id,
                    "native-extra",
                    &CancelToken::new(),
                    &crate::MockRunOptions::default(),
                )
                .unwrap();
            runtime
                .set_session_harness(&id, HarnessKind::Codex)
                .unwrap();
            runtime
                .run_foreground(&id, "back-to-codex", &CancelToken::new(), &options)
                .unwrap();
            let invocations = std::fs::read_to_string(format!("{root}/invocations")).unwrap();
            assert_eq!(invocations.matches("first-message").count(), 1);
            assert_eq!(invocations.matches("native-extra").count(), 1);
            assert!(
                runtime
                    .run_foreground(&id, "trigger-failure", &CancelToken::new(), &options)
                    .is_err()
            );
            let projection = runtime.replay_session(&id).unwrap();
            assert_eq!(
                projection.runs.last().unwrap().status,
                blora_types::RunStatus::Failed
            );
            assert!(projection.external_sessions.is_empty());
            // Failed context is synced into a fresh external session on retry.
            std::fs::write(format!("{root}/recover"), "").unwrap();
            runtime
                .run_foreground(&id, "retry-message", &CancelToken::new(), &options)
                .unwrap();
            let unsupported = RunOptions {
                permission: Some(crate::PermissionMode::AutoEdit),
                ..Default::default()
            };
            let before = std::fs::read_to_string(format!("{root}/invocations")).unwrap();
            assert!(
                runtime
                    .run_foreground(&id, "unsupported", &CancelToken::new(), &unsupported)
                    .unwrap_err()
                    .to_string()
                    .contains("unsupported")
            );
            assert_eq!(
                before,
                std::fs::read_to_string(format!("{root}/invocations")).unwrap()
            );
            let cancel = CancelToken::new();
            let other = cancel.clone();
            let thread = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(150));
                other.cancel();
            });
            assert!(matches!(
                runtime.run_foreground(&id, "trigger-sleep", &cancel, &options),
                Err(BloraError::Cancelled)
            ));
            thread.join().unwrap();
            assert_eq!(
                runtime
                    .replay_session(&id)
                    .unwrap()
                    .runs
                    .last()
                    .unwrap()
                    .status,
                blora_types::RunStatus::Cancelled
            );
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("codex");
        std::fs::write(&fake, r#"#!/bin/sh
printf '%s\n' "$@" >> "$BLORA_RUNTIME_FAKE_ROOT/invocations"
case "$*" in *trigger-failure*) if [ ! -f "$BLORA_RUNTIME_FAKE_ROOT/recover" ]; then exit 7; fi ;; esac
case "$*" in *trigger-sleep*) /bin/sleep 10 ;; esac
printf '%s\n' '{"type":"thread.started","thread_id":"thread-1"}' '{"type":"item.completed","item":{"id":"reason-1","type":"reasoning","text":"thinking"}}' '{"type":"item.completed","item":{"id":"tool-1","type":"command_execution","command":"pwd","aggregated_output":"ok","exit_code":0}}' '{"type":"item.updated","item":{"id":"reply-1","type":"agent_message","text":"hello"}}' '{"type":"item.completed","item":{"id":"reply-1","type":"agent_message","text":"hello world"}}' '{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":4}}'
"#).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("/bin/kill", dir.path().join("kill")).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "external::tests::fake_cli_lifecycle_resume_queue_failure_and_cancel",
                "--nocapture",
            ])
            .env("BLORA_RUNTIME_FAKE_ROOT", dir.path())
            .env("PATH", dir.path())
            .status()
            .unwrap();
        assert!(status.success());
    }
    #[test]
    fn harness_binding_survives_store_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        let runtime = Runtime::new(SqliteStore::open(&path).unwrap());
        let id = session(&runtime, dir.path().to_str().unwrap());
        runtime
            .emit(
                &id,
                None,
                None,
                KnownPayload::HarnessChanged(HarnessChanged {
                    harness: HarnessKind::GrokBuild,
                }),
            )
            .unwrap();
        runtime
            .emit(
                &id,
                None,
                None,
                KnownPayload::ExternalSessionBound(ExternalSessionBound {
                    harness: HarnessKind::GrokBuild,
                    external_session_id: "explicit-id".into(),
                    cwd: dir.path().to_str().unwrap().into(),
                    synced_sequence: 2,
                }),
            )
            .unwrap();
        drop(runtime);
        let runtime = Runtime::new(SqliteStore::open(&path).unwrap());
        let projection = runtime.replay_session(&id).unwrap();
        assert_eq!(projection.harness, HarnessKind::GrokBuild);
        assert_eq!(
            projection.external_sessions[0].external_session_id,
            "explicit-id"
        );
        let other = session(&runtime, dir.path().to_str().unwrap());
        assert!(
            runtime
                .replay_session(&other)
                .unwrap()
                .external_sessions
                .is_empty()
        );
    }
    #[test]
    fn concurrent_same_name_tools_complete_by_call_id() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let id = session(&runtime, "/tmp");
        let run = RunId::generate();
        let turn = TurnId::generate();
        runtime
            .emit(
                &id,
                Some(&run),
                Some(&turn),
                KnownPayload::RunCreated(RunCreated {
                    mode: Mode::Code,
                    model: None,
                    permission_mode: None,
                }),
            )
            .unwrap();
        runtime
            .emit(
                &id,
                Some(&run),
                Some(&turn),
                KnownPayload::RunStarted(RunStarted { model: None }),
            )
            .unwrap();
        for call in ["a", "b"] {
            runtime
                .emit(
                    &id,
                    Some(&run),
                    Some(&turn),
                    KnownPayload::ToolRequested(ToolRequested {
                        tool: "shell".into(),
                        arguments: serde_json::json!({}),
                        call_id: Some(call.into()),
                    }),
                )
                .unwrap();
            runtime
                .emit_external_tool(
                    &id,
                    &run,
                    &turn,
                    call,
                    KnownPayload::ToolStarted(ToolStarted {
                        tool: "shell".into(),
                    }),
                )
                .unwrap();
        }
        runtime
            .emit_external_tool(
                &id,
                &run,
                &turn,
                "a",
                KnownPayload::ToolCompleted(ToolCompleted {
                    tool: "shell".into(),
                    ok: true,
                }),
            )
            .unwrap();
        let projection = runtime.show_session(&id).unwrap();
        for item in projection.transcript {
            if let blora_session::TranscriptItem::Tool {
                call_id: Some(call),
                status,
                ..
            } = item
            {
                assert_eq!(status, if call == "a" { "completed" } else { "running" });
            }
        }
    }
    #[test]
    fn queue_cancel_after_success_restores_unsent_messages() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let id = session(&runtime, "/tmp");
        for message in ["first", "second", "third"] {
            runtime.queue_steer(&id, message).unwrap();
        }
        let cancel = CancelToken::new();
        let mut calls = 0;
        let result = runtime.drain_external_queue(&id, &cancel, |message| {
            calls += 1;
            runtime
                .emit(
                    &id,
                    None,
                    None,
                    KnownPayload::UserInput(UserInput {
                        text: message.into(),
                    }),
                )
                .unwrap();
            cancel.cancel();
            Ok(RunId::generate())
        });
        assert!(matches!(result, Err(BloraError::Cancelled)));
        assert_eq!(calls, 1);
        assert_eq!(runtime.store.take_steers(&id).unwrap(), ["second", "third"]);
    }

    #[test]
    fn queue_cancel_before_input_restores_current_message() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let id = session(&runtime, "/tmp");
        runtime.queue_steer(&id, "first").unwrap();
        runtime.queue_steer(&id, "second").unwrap();
        let cancel = CancelToken::new();
        assert!(
            runtime
                .drain_external_queue(&id, &cancel, |_| {
                    cancel.cancel();
                    Err(BloraError::Cancelled)
                })
                .is_err()
        );
        assert_eq!(runtime.store.take_steers(&id).unwrap(), ["first", "second"]);
    }

    #[test]
    fn warning_is_visible_status_not_reasoning_or_context() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let id = session(&runtime, "/tmp");
        let run = RunId::generate();
        let turn = TurnId::generate();
        runtime
            .emit(
                &id,
                Some(&run),
                Some(&turn),
                KnownPayload::RunCreated(RunCreated {
                    mode: Mode::Code,
                    model: None,
                    permission_mode: None,
                }),
            )
            .unwrap();
        let mut seen = HashSet::new();
        let mut warnings = Vec::new();
        collect_warning(&mut seen, &mut warnings, "permission denied".into());
        collect_warning(&mut seen, &mut warnings, "permission denied".into());
        assert_eq!(warnings.len(), 1);
        for warning in warnings {
            runtime
                .emit_external_warning(&id, &run, &turn, HarnessKind::Codex, &warning)
                .unwrap();
        }
        let events = runtime.events(&id).unwrap();
        let warning = events.last().unwrap();
        assert_eq!(warning.event_type, "assistant.message.completed");
        assert_eq!(warning.actor, blora_types::Actor::System);
        assert!(
            warning.payload["text"]
                .as_str()
                .unwrap()
                .contains("状态提示")
        );
        assert_eq!(context_prompt(&events, 0, "next"), "next");
    }

    #[test]
    fn context_history_is_bounded_utf8_tail() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let id = session(&runtime, "/tmp");
        runtime
            .emit(
                &id,
                None,
                None,
                KnownPayload::UserInput(UserInput {
                    text: "旧".repeat(100_000),
                }),
            )
            .unwrap();
        runtime
            .emit(
                &id,
                None,
                None,
                KnownPayload::UserInput(UserInput {
                    text: "recent request".into(),
                }),
            )
            .unwrap();
        let prompt = context_prompt(&runtime.events(&id).unwrap(), 0, "current");
        assert!(prompt.len() < 66 * 1024);
        assert!(prompt.contains("recent request"));
        assert!(prompt.contains("omitted"));
        assert!(prompt.ends_with("current"));
    }
}
