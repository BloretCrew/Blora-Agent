// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use blora_content::AttachmentRef;
use blora_events::KnownPayload;
use blora_runtime::PermissionMode;
use blora_types::{ApprovalId, BloraError, CancelToken, RunId, RunStatus, SessionId};
use serde_json::Value;

use crate::agent::AgentInner;
use crate::error::{Result, SdkError};
use crate::events::EventStream;

/// One prompt submission.
#[derive(Clone, Debug, Default)]
pub struct RunRequest {
    pub prompt: String,
    pub attachments: Vec<AttachmentRef>,
    pub output_schema: Option<Value>,
    pub model: Option<String>,
    pub max_turns: Option<u32>,
    pub permission: Option<PermissionMode>,
}

impl RunRequest {
    #[must_use]
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn attachments(mut self, attachments: Vec<AttachmentRef>) -> Self {
        self.attachments = attachments;
        self
    }

    #[must_use]
    pub fn output_schema(mut self, schema: Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    #[must_use]
    pub fn max_turns(mut self, turns: u32) -> Self {
        self.max_turns = Some(turns);
        self
    }

    #[must_use]
    pub fn permission(mut self, permission: PermissionMode) -> Self {
        self.permission = Some(permission);
        self
    }
}

/// Token usage for one run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
}

/// Final result of a run.
#[derive(Debug)]
pub struct RunOutcome {
    pub run_id: RunId,
    pub status: RunStatus,
    pub final_text: String,
    pub usage: Usage,
    pub error: Option<SdkError>,
    pub last_sequence: u64,
}

/// Handle to a running agent turn.
pub struct RunHandle {
    inner: Arc<AgentInner>,
    session_id: SessionId,
    run_id: RunId,
    start_sequence: u64,
    cancel: CancelToken,
    done: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<std::result::Result<RunId, BloraError>>>,
    detached: bool,
}

impl RunHandle {
    pub(crate) fn new(
        inner: Arc<AgentInner>,
        session_id: SessionId,
        run_id: RunId,
        start_sequence: u64,
        cancel: CancelToken,
        done: Arc<AtomicBool>,
        join: std::thread::JoinHandle<std::result::Result<RunId, BloraError>>,
    ) -> Self {
        Self {
            inner,
            session_id,
            run_id,
            start_sequence,
            cancel,
            done,
            join: Some(join),
            detached: false,
        }
    }

    #[must_use]
    pub fn id(&self) -> &RunId {
        &self.run_id
    }

    #[must_use]
    pub fn start_sequence(&self) -> u64 {
        self.start_sequence
    }

    /// Let the run continue if this handle is dropped.
    pub fn detach(&mut self) {
        self.detached = true;
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// A clone of the run's cancellation token.
    #[must_use]
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    pub fn steer(&self, message: &str) -> Result<()> {
        self.inner.runtime.queue_steer(&self.session_id, message)?;
        Ok(())
    }

    pub fn approve(&self, approval_id: &ApprovalId, allow: bool) -> Result<()> {
        self.inner.runtime.resolve_approval(approval_id, allow)?;
        Ok(())
    }

    pub fn answer(&self, request_id: &str, answer: Option<String>) -> Result<()> {
        self.inner
            .runtime
            .answer_input(&self.session_id, request_id, answer)?;
        Ok(())
    }

    /// Events for this run, starting at its first sequence.
    pub fn events(&self) -> EventStream {
        EventStream::new(
            self.inner.runtime.clone(),
            self.session_id.clone(),
            self.start_sequence,
            Some(self.inner.runtime.subscribe_session(&self.session_id)),
            Some(self.run_id.clone()),
            Some(self.done.clone()),
        )
    }

    pub fn wait(mut self) -> Result<RunOutcome> {
        let join = self
            .join
            .take()
            .ok_or_else(|| SdkError::Internal("run handle already consumed".to_owned()))?;
        let joined = join
            .join()
            .map_err(|_| SdkError::Internal("run thread panicked".to_owned()))?;
        self.outcome(joined)
    }

    pub fn wait_timeout(self, timeout: Duration) -> Result<RunOutcome> {
        let started = Instant::now();
        loop {
            let finished = self.join.as_ref().is_none_or(|join| join.is_finished());
            if finished {
                break;
            }
            if started.elapsed() >= timeout {
                self.cancel.cancel();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.wait()
    }

    fn outcome(&self, joined: std::result::Result<RunId, BloraError>) -> Result<RunOutcome> {
        let events = self
            .inner
            .runtime
            .events_after(&self.session_id, self.start_sequence)
            .map_err(SdkError::Kernel)?;
        let mut status = RunStatus::Running;
        let mut final_text = String::new();
        let mut usage = Usage::default();
        let mut error = None;
        let mut last_sequence = self.start_sequence;
        for event in &events {
            if event.run_id.as_ref() != Some(&self.run_id) {
                continue;
            }
            last_sequence = event.sequence;
            let Ok(Some(payload)) = KnownPayload::from_type(&event.event_type, &event.payload)
            else {
                continue;
            };
            match payload {
                KnownPayload::AssistantMessageCompleted(message) => {
                    final_text = message.text;
                }
                KnownPayload::UsageRecorded(recorded) => {
                    usage.input_tokens += recorded.input_tokens;
                    usage.output_tokens += recorded.output_tokens;
                    usage.cached_tokens += recorded.cached_tokens;
                }
                KnownPayload::RunCompleted(_) => status = RunStatus::Completed,
                KnownPayload::RunFailed(failed) => {
                    status = RunStatus::Failed;
                    error = Some(SdkError::Kernel(BloraError::Other(failed.error)));
                }
                KnownPayload::RunCancelled(_) => status = RunStatus::Cancelled,
                _ => {}
            }
        }
        if let Err(err) = joined {
            if status == RunStatus::Running {
                status = if matches!(err, BloraError::Cancelled) {
                    RunStatus::Cancelled
                } else {
                    RunStatus::Failed
                };
            }
            if error.is_none() && !matches!(err, BloraError::Cancelled) {
                error = Some(SdkError::Kernel(err));
            }
        }
        // A run that produced a terminal event but whose thread returned Ok is done.
        if status == RunStatus::Running {
            status = RunStatus::Failed;
            error.get_or_insert_with(|| {
                SdkError::Internal("run ended without a terminal event".to_owned())
            });
        }
        Ok(RunOutcome {
            run_id: self.run_id.clone(),
            status,
            final_text,
            usage,
            error,
            last_sequence,
        })
    }
}

impl std::fmt::Debug for RunHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunHandle")
            .field("run_id", &self.run_id)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl Drop for RunHandle {
    fn drop(&mut self) {
        if !self.detached {
            self.cancel.cancel();
        }
        let _ = self.done.load(Ordering::SeqCst);
    }
}
