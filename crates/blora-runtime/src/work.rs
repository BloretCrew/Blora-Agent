// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{KnownPayload, TaskCompleted, TaskCreated, TaskFailed, TaskStarted, TaskWakeup};
use blora_storage::{CreateTask, TaskRecord};
use blora_types::{BloraError, CancelToken, Result, RunId, SessionId, TaskId, TaskStatus};
use chrono::{Duration, Utc};

use crate::coordinator::{RunOptions, Runtime};

/// Hard wall-clock cap for a single scheduled task run (seconds).
const TASK_WALL_SECS: u64 = 180;
/// Retry backoff: 10s, 20s, 40s ... capped at 15 minutes.
const RETRY_BASE_SECS: i64 = 10;
const RETRY_CAP_SECS: i64 = 900;

impl Runtime {
    pub fn create_task(&self, spec: CreateTask) -> Result<TaskId> {
        if spec.prompt.trim().is_empty() {
            return Err(BloraError::Other(
                "task prompt must not be empty".to_owned(),
            ));
        }
        if let Some(cron) = &spec.cron {
            crate::next_cron(cron, Utc::now())?;
        }
        let task_id = TaskId::generate();
        let now = Utc::now();
        self.store.insert_task(&task_id, &spec, now)?;
        self.emit(
            &spec.session_id,
            None,
            None,
            KnownPayload::TaskCreated(TaskCreated {
                task_id: task_id.clone(),
                title: spec.title,
                prompt: Some(spec.prompt),
                delay_until: spec.delay_until.map(|time| time.to_rfc3339()),
            }),
        )?;
        Ok(task_id)
    }

    pub fn list_tasks(&self, session_id: Option<&SessionId>) -> Result<Vec<TaskRecord>> {
        self.store.list_tasks(session_id)
    }

    pub fn get_task(&self, task_id: &TaskId) -> Result<TaskRecord> {
        self.store.get_task(task_id)
    }

    pub fn pause_task(&self, task_id: &TaskId) -> Result<()> {
        let now = Utc::now();
        if !self.store.cas_task_status(
            task_id,
            &[TaskStatus::Queued, TaskStatus::Scheduled],
            TaskStatus::Paused,
            now,
            Some("paused"),
            None,
        )? {
            return Err(BloraError::Other(format!(
                "task {task_id} cannot be paused"
            )));
        }
        Ok(())
    }

    pub fn resume_task(&self, task_id: &TaskId) -> Result<()> {
        let now = Utc::now();
        if !self.store.cas_task_status(
            task_id,
            &[TaskStatus::Paused],
            TaskStatus::Queued,
            now,
            None,
            None,
        )? {
            return Err(BloraError::Other(format!(
                "task {task_id} cannot be resumed"
            )));
        }
        Ok(())
    }

    pub fn cancel_task(&self, task_id: &TaskId) -> Result<()> {
        let task = self.store.get_task(task_id)?;
        if task.status.is_terminal() {
            return Ok(());
        }
        let now = Utc::now();
        self.store.cas_task_status(
            task_id,
            &[
                TaskStatus::Queued,
                TaskStatus::Scheduled,
                TaskStatus::Running,
                TaskStatus::Paused,
            ],
            TaskStatus::Cancelled,
            now,
            Some("cancelled"),
            None,
        )?;
        Ok(())
    }

    /// Run every due task once. Claiming is compare-and-swap so two pumps never
    /// execute the same occurrence; a crash mid-run leaves the task `running`,
    /// which `recover_stale_tasks` turns into a retry on the next pump.
    pub fn pump(&self) -> Result<Vec<TaskId>> {
        self.recover_stale_tasks()?;
        let claimed = self.store.claim_due_tasks(Utc::now())?;
        let mut finished = Vec::new();
        for task in claimed {
            match self.execute_task(&task) {
                Ok(()) => finished.push(task.id),
                Err(err) => {
                    tracing::warn!("task {} failed: {err}", task.id);
                    finished.push(task.id);
                }
            }
        }
        Ok(finished)
    }

    /// Tasks left `running` for longer than the wall-clock cap are assumed to have
    /// died with their process. They are rescheduled (or failed) exactly once.
    fn recover_stale_tasks(&self) -> Result<()> {
        let cutoff = Utc::now() - Duration::seconds(TASK_WALL_SECS as i64 * 2);
        for task in self.store.list_tasks(None)? {
            if task.status != TaskStatus::Running || task.updated_at > cutoff {
                continue;
            }
            if task.attempt < task.max_attempts {
                self.store.reschedule_task(
                    &task.id,
                    Utc::now(),
                    Utc::now(),
                    "recovered stale running task",
                )?;
            } else {
                self.finish_task(&task, TaskStatus::Failed, Some("stale run abandoned"), None)?;
            }
        }
        Ok(())
    }

    fn execute_task(&self, task: &TaskRecord) -> Result<()> {
        self.emit(
            &task.session_id,
            None,
            None,
            KnownPayload::TaskStarted(TaskStarted {
                task_id: task.id.clone(),
            }),
        )?;
        // Advance the cron schedule *before* dispatch so a crash mid-run cannot
        // re-fire the same occurrence (at-most-once), then run.
        let next_fire = task
            .cron
            .as_deref()
            .and_then(|cron| crate::next_cron(cron, Utc::now()).ok());
        let prompt = format!("[background-task] {}\n\n{}", task.title, task.prompt);
        let options = RunOptions {
            mock: task.mock,
            auto_approve: task.auto_approve,
            max_turns: 8,
            ..RunOptions::default()
        };
        let cancel = CancelToken::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(TASK_WALL_SECS);
        let watchdog_cancel = cancel.clone();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let watchdog = std::thread::spawn(move || {
            while std::time::Instant::now() < deadline {
                if stop_rx
                    .recv_timeout(std::time::Duration::from_millis(250))
                    .is_ok()
                {
                    return;
                }
            }
            watchdog_cancel.cancel();
        });
        let outcome = self.run(&task.session_id, &prompt, &cancel, &options);
        let _ = stop_tx.send(());
        let _ = watchdog.join();
        match outcome {
            Ok(run_id) => {
                self.finish_task(task, TaskStatus::Completed, None, Some(&run_id))?;
                if let Some(next) = next_fire {
                    let _ =
                        self.store
                            .reschedule_task(&task.id, next, Utc::now(), "cron next fire");
                    let _ = self.store.set_task_attempt(&task.id, 0, Utc::now());
                }
                Ok(())
            }
            Err(BloraError::Cancelled) => {
                let reason = if cancel.is_cancelled() {
                    "wall-clock limit reached"
                } else {
                    "cancelled"
                };
                if let Some(next) = next_fire {
                    self.finish_task(task, TaskStatus::Failed, Some(reason), None)?;
                    let _ = self
                        .store
                        .reschedule_task(&task.id, next, Utc::now(), reason);
                } else {
                    let _ = self.store.cas_task_status(
                        &task.id,
                        &[TaskStatus::Running],
                        TaskStatus::Cancelled,
                        Utc::now(),
                        Some(reason),
                        None,
                    );
                }
                Ok(())
            }
            Err(err) => {
                let now = Utc::now();
                if task.attempt < task.max_attempts {
                    let exp = RETRY_BASE_SECS
                        .saturating_mul(1i64 << task.attempt.min(10))
                        .min(RETRY_CAP_SECS);
                    self.store.reschedule_task(
                        &task.id,
                        now + Duration::seconds(exp),
                        now,
                        &err.to_string(),
                    )?;
                    self.emit(
                        &task.session_id,
                        None,
                        None,
                        KnownPayload::TaskFailed(TaskFailed {
                            task_id: task.id.clone(),
                            error: format!("retry in {exp}s: {err}"),
                        }),
                    )?;
                } else if let Some(next) = next_fire {
                    self.finish_task(task, TaskStatus::Failed, Some(&err.to_string()), None)?;
                    let _ = self
                        .store
                        .reschedule_task(&task.id, next, now, &err.to_string());
                    let _ = self.store.set_task_attempt(&task.id, 0, now);
                } else {
                    self.finish_task(task, TaskStatus::Failed, Some(&err.to_string()), None)?;
                }
                Ok(())
            }
        }
    }

    fn finish_task(
        &self,
        task: &TaskRecord,
        status: TaskStatus,
        error: Option<&str>,
        run_id: Option<&RunId>,
    ) -> Result<()> {
        let now = Utc::now();
        self.store
            .cas_task_status(&task.id, &[TaskStatus::Running], status, now, error, run_id)?;
        match status {
            TaskStatus::Completed => {
                let summary = self
                    .show_session(&task.session_id)
                    .ok()
                    .and_then(|projection| {
                        projection.transcript.into_iter().rev().find_map(|item| {
                            if let blora_session::TranscriptItem::Assistant { text, .. } = item {
                                Some(text)
                            } else {
                                None
                            }
                        })
                    });
                self.emit(
                    &task.session_id,
                    None,
                    None,
                    KnownPayload::TaskCompleted(TaskCompleted {
                        task_id: task.id.clone(),
                        summary,
                    }),
                )?;
                self.emit(
                    &task.session_id,
                    None,
                    None,
                    KnownPayload::TaskWakeup(TaskWakeup {
                        task_id: task.id.clone(),
                    }),
                )?;
            }
            TaskStatus::Failed => {
                self.emit(
                    &task.session_id,
                    None,
                    None,
                    KnownPayload::TaskFailed(TaskFailed {
                        task_id: task.id.clone(),
                        error: error.unwrap_or("failed").to_owned(),
                    }),
                )?;
            }
            _ => {}
        }
        Ok(())
    }
}
