// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{
    AssistantDelta, AssistantMessageCompleted, KnownPayload, ModelRequested,
    ModelResponseCompleted, NewEvent, RunCancelRequested, RunCancelled, RunCompleted, RunCreated,
    RunStarted, UsageRecorded, UserInput,
};
use blora_session::SessionProjection;
use blora_storage::{CreateSession, SessionSummary, SqliteStore};
use blora_types::{BloraError, Mode, Result, RunId, SessionId, TurnId};

use crate::CancelToken;

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
        let mode = self
            .store
            .load_projection(session_id)?
            .session
            .as_ref()
            .map(|session| session.mode)
            .unwrap_or(Mode::Code);

        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunCreated(RunCreated {
                mode,
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
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::ModelRequested(ModelRequested {
                provider: "mock".to_owned(),
                model: options.model.clone(),
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
            KnownPayload::ModelResponseCompleted(ModelResponseCompleted {
                finish_reason: Some("stop".to_owned()),
            }),
        )?;
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::UsageRecorded(UsageRecorded {
                input_tokens: u64::try_from(prompt.len()).unwrap_or(0),
                output_tokens: u64::try_from(options.chunks.iter().map(String::len).sum::<usize>())
                    .unwrap_or(0),
                cached_tokens: 0,
            }),
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
                workspace_path: "/tmp/ws".to_owned(),
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
                workspace_path: "/tmp/ws".to_owned(),
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
}
