// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use blora_events::EventEnvelope;
use blora_runtime::Runtime;
use blora_types::{RunId, SessionId};

use crate::error::{Result, SdkError};

/// A lossless, ordered event stream backed by the canonical event log.
///
/// The live subscription is only a wake-up signal. Missing or dropped
/// notifications are recovered by reading `events_after(next_sequence)` from
/// the store, so a slow consumer sees every event exactly once in order.
pub struct EventStream {
    runtime: Arc<Runtime>,
    session_id: SessionId,
    live: Option<std::sync::mpsc::Receiver<EventEnvelope>>,
    next_sequence: u64,
    run_filter: Option<RunId>,
    buffer: VecDeque<EventEnvelope>,
    done: Option<Arc<AtomicBool>>,
}

impl EventStream {
    pub(crate) fn new(
        runtime: Arc<Runtime>,
        session_id: SessionId,
        after_sequence: u64,
        live: Option<std::sync::mpsc::Receiver<EventEnvelope>>,
        run_filter: Option<RunId>,
        done: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            runtime,
            session_id,
            live,
            next_sequence: after_sequence,
            run_filter,
            buffer: VecDeque::new(),
            done,
        }
    }

    /// Next event, blocking until one is available or the run finishes.
    pub fn next_event(&mut self) -> Result<Option<EventEnvelope>> {
        loop {
            if let Some(event) = self.next_timeout(Duration::from_millis(250))? {
                return Ok(Some(event));
            }
            if self.finished() {
                return Ok(None);
            }
        }
    }

    /// Next event, or `None` after `timeout`.
    pub fn next_timeout(&mut self, timeout: Duration) -> Result<Option<EventEnvelope>> {
        if let Some(event) = self.buffer.pop_front() {
            return Ok(Some(event));
        }
        self.refill()?;
        if let Some(event) = self.buffer.pop_front() {
            return Ok(Some(event));
        }
        let Some(live) = self.live.as_ref() else {
            return Ok(None);
        };
        match live.recv_timeout(timeout) {
            Ok(_) => {
                self.refill()?;
                Ok(self.buffer.pop_front())
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                self.refill()?;
                Ok(self.buffer.pop_front())
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                self.live = None;
                self.refill()?;
                Ok(self.buffer.pop_front())
            }
        }
    }

    /// Non-blocking poll.
    pub fn try_next(&mut self) -> Result<Option<EventEnvelope>> {
        if let Some(event) = self.buffer.pop_front() {
            return Ok(Some(event));
        }
        self.refill()?;
        Ok(self.buffer.pop_front())
    }

    fn refill(&mut self) -> Result<()> {
        let events = self
            .runtime
            .events_after(&self.session_id, self.next_sequence)
            .map_err(SdkError::Kernel)?;
        for event in events {
            if event.sequence <= self.next_sequence {
                continue;
            }
            self.next_sequence = event.sequence;
            let matches = self.run_filter.as_ref().is_none_or(|run_id| {
                event.run_id.as_ref() == Some(run_id) || event.run_id.is_none()
            });
            if matches {
                self.buffer.push_back(event);
            }
        }
        Ok(())
    }

    fn finished(&self) -> bool {
        self.live.is_none()
            || self
                .done
                .as_ref()
                .is_some_and(|done| done.load(Ordering::SeqCst))
    }
}
