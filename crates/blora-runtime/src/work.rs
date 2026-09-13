// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{KnownPayload, TaskCompleted, TaskCreated, TaskFailed, TaskStarted, TaskWakeup};
use blora_storage::{CreateTask, TaskRecord};
use blora_types::{BloraError, CancelToken, Result, RunId, SessionId, TaskId, TaskStatus};
use chrono::{Duration, Utc};

use crate::coordinator::{RunOptions, Runtime};

impl Runtime {
    pub fn create_task(&self, spec: CreateTask) -> Result<TaskId> {
        if spec.prompt.trim().is_empty() {
            return Err(BloraError::Other(
                "task prompt must not be empty".to_owned(),
            ));
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

    pub fn pump(&self) -> Result<Vec<TaskId>> {
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

    fn execute_task(&self, task: &TaskRecord) -> Result<()> {
        self.emit(
            &task.session_id,
            None,
            None,
            KnownPayload::TaskStarted(TaskStarted {
                task_id: task.id.clone(),
            }),
        )?;
        let prompt = format!("[background-task] {}\n\n{}", task.title, task.prompt);
        let options = RunOptions {
            mock: task.mock,
            auto_approve: task.auto_approve,
            max_turns: 8,
            ..RunOptions::default()
        };
        match self.run(&task.session_id, &prompt, &CancelToken::new(), &options) {
            Ok(run_id) => {
                self.finish_task(task, TaskStatus::Completed, None, Some(&run_id))?;
                if let Some(cron) = &task.cron {
                    if let Ok(next) = crate::next_cron(cron, Utc::now()) {
                        let _ = self.store.reschedule_task(
                            &task.id,
                            next,
                            Utc::now(),
                            "cron next fire",
                        );
                    }
                }
                Ok(())
            }
            Err(BloraError::Cancelled) => {
                let _ = self.store.cas_task_status(
                    &task.id,
                    &[TaskStatus::Running],
                    TaskStatus::Cancelled,
                    Utc::now(),
                    Some("cancelled"),
                    None,
                );
                Ok(())
            }
            Err(err) => {
                let now = Utc::now();
                if task.attempt < task.max_attempts {
                    self.store.reschedule_task(
                        &task.id,
                        now + Duration::seconds(10),
                        now,
                        &err.to_string(),
                    )?;
                    self.emit(
                        &task.session_id,
                        None,
                        None,
                        KnownPayload::TaskFailed(TaskFailed {
                            task_id: task.id.clone(),
                            error: format!("retry scheduled: {err}"),
                        }),
                    )?;
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
