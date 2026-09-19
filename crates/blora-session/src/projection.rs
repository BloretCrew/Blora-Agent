// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{EventEnvelope, KnownPayload};
use blora_types::{
    AgentId, ApprovalId, BloraError, EventId, Mode, Result, RunId, RunStatus, SessionId,
    SessionStatus, TaskId, TaskStatus, TurnId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::machine::transition_run;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub title: Option<String>,
    pub workspace_path: String,
    pub mode: Mode,
    pub status: SessionStatus,
    pub parent_session_id: Option<SessionId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: RunId,
    pub session_id: SessionId,
    pub status: RunStatus,
    pub mode: Mode,
    pub model: Option<String>,
    pub turn_id: Option<TurnId>,
    pub cancel_requested: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TranscriptItem {
    User {
        text: String,
        event_id: EventId,
    },
    Assistant {
        text: String,
        event_id: EventId,
    },
    Tool {
        name: String,
        status: String,
        event_id: EventId,
        #[serde(default)]
        arguments: Option<String>,
        #[serde(default)]
        output: Option<String>,
        #[serde(default)]
        call_id: Option<String>,
    },
    System {
        summary: String,
        event_id: EventId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskView {
    pub id: TaskId,
    pub title: String,
    pub status: TaskStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentView {
    pub id: AgentId,
    pub role: String,
    pub child_session_id: Option<SessionId>,
    pub status: String,
    pub summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovalView {
    pub id: ApprovalId,
    pub summary: String,
    pub status: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionProjection {
    pub session: Option<SessionRecord>,
    pub runs: Vec<RunRecord>,
    pub transcript: Vec<TranscriptItem>,
    pub tasks: Vec<TaskView>,
    pub subagents: Vec<SubagentView>,
    pub approvals: Vec<ApprovalView>,
    pub last_sequence: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(skip)]
    open_assistant: String,
}

impl SessionProjection {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: &EventEnvelope) -> Result<()> {
        apply_event(self, event)
    }
}

pub fn rebuild(events: &[EventEnvelope]) -> Result<SessionProjection> {
    let mut projection = SessionProjection::new();
    for event in events {
        apply_event(&mut projection, event)?;
    }
    Ok(projection)
}

pub fn apply_event(projection: &mut SessionProjection, event: &EventEnvelope) -> Result<()> {
    event.validate()?;
    let expected = projection.last_sequence + 1;
    if event.sequence != expected {
        return Err(BloraError::SequenceMismatch {
            expected,
            actual: event.sequence,
        });
    }
    projection.last_sequence = event.sequence;

    let Some(payload) = event.decode_payload()? else {
        return Ok(());
    };

    match payload {
        KnownPayload::SessionCreated(created) => {
            if projection.session.is_some() {
                return Err(BloraError::event("session.created emitted twice"));
            }
            let record = SessionRecord {
                id: event.session_id.clone(),
                title: created.title,
                workspace_path: created.workspace_path,
                mode: created.mode,
                status: SessionStatus::Active,
                parent_session_id: created.parent_session_id,
                created_at: event.timestamp,
                updated_at: event.timestamp,
            };
            projection.session = Some(record);
        }
        KnownPayload::SessionResumed(_) => touch_session(projection, event.timestamp)?,
        KnownPayload::SessionForked(forked) => {
            if let Some(session) = projection.session.as_mut() {
                session.parent_session_id = Some(forked.source_session_id);
                session.updated_at = event.timestamp;
            }
        }
        KnownPayload::SessionArchived(_) => {
            let session = session_mut(projection)?;
            session.status = SessionStatus::Archived;
            session.updated_at = event.timestamp;
        }
        KnownPayload::RunCreated(created) => {
            let run_id = event
                .run_id
                .clone()
                .ok_or_else(|| BloraError::event("run.created requires run_id"))?;
            projection.runs.push(RunRecord {
                id: run_id,
                session_id: event.session_id.clone(),
                status: RunStatus::Queued,
                mode: created.mode,
                model: created.model,
                turn_id: event.turn_id.clone(),
                cancel_requested: false,
                created_at: event.timestamp,
                updated_at: event.timestamp,
            });
        }
        KnownPayload::RunCancelRequested(_) => {
            let run = run_mut(projection, event)?;
            run.cancel_requested = true;
            run.updated_at = event.timestamp;
        }
        KnownPayload::UserInput(input) => {
            projection.transcript.push(TranscriptItem::User {
                text: input.text,
                event_id: event.event_id.clone(),
            });
            touch_run(projection, event)?;
        }
        KnownPayload::AssistantDelta(delta) => {
            projection.open_assistant.push_str(&delta.text);
            touch_run(projection, event)?;
        }
        KnownPayload::AssistantMessageCompleted(completed) => {
            let text = if completed.text.is_empty() {
                std::mem::take(&mut projection.open_assistant)
            } else {
                projection.open_assistant.clear();
                completed.text
            };
            projection.transcript.push(TranscriptItem::Assistant {
                text,
                event_id: event.event_id.clone(),
            });
            touch_run(projection, event)?;
        }
        KnownPayload::ToolRequested(tool) => {
            let arguments = compact_tool_args(&tool.arguments);
            if let Some(TranscriptItem::Tool {
                status,
                arguments: slot,
                call_id,
                event_id,
                ..
            }) = find_tool_mut(projection, Some(&tool.tool), tool.call_id.as_deref())
            {
                *status = "requested".to_owned();
                if arguments.is_some() {
                    *slot = arguments;
                }
                if call_id.is_none() {
                    *call_id = tool.call_id;
                }
                *event_id = event.event_id.clone();
            } else {
                projection.transcript.push(TranscriptItem::Tool {
                    name: tool.tool,
                    status: "requested".to_owned(),
                    event_id: event.event_id.clone(),
                    arguments,
                    output: None,
                    call_id: tool.call_id,
                });
            }
            touch_run(projection, event)?;
        }
        KnownPayload::ToolStarted(tool) => {
            if let Some(TranscriptItem::Tool { status, .. }) =
                find_tool_mut(projection, Some(&tool.tool), None)
            {
                *status = "running".to_owned();
            }
            touch_run(projection, event)?;
        }
        KnownPayload::ToolOutput(output) => {
            if let Some(TranscriptItem::Tool {
                output: slot,
                status,
                ..
            }) = find_tool_mut(projection, None, output.call_id.as_deref())
            {
                *slot = Some(output.text);
                if status == "requested" {
                    *status = "running".to_owned();
                }
            } else {
                projection.transcript.push(TranscriptItem::Tool {
                    name: "tool".to_owned(),
                    status: "output".to_owned(),
                    event_id: event.event_id.clone(),
                    arguments: None,
                    output: Some(output.text),
                    call_id: output.call_id,
                });
            }
            touch_run(projection, event)?;
        }
        KnownPayload::ToolCompleted(tool) => {
            let status = if tool.ok { "completed" } else { "failed" };
            if let Some(TranscriptItem::Tool {
                status: slot,
                event_id,
                ..
            }) = find_tool_mut(projection, Some(&tool.tool), None)
            {
                *slot = status.to_owned();
                *event_id = event.event_id.clone();
            } else {
                projection.transcript.push(TranscriptItem::Tool {
                    name: tool.tool,
                    status: status.to_owned(),
                    event_id: event.event_id.clone(),
                    arguments: None,
                    output: None,
                    call_id: None,
                });
            }
            touch_run(projection, event)?;
        }
        KnownPayload::ToolFailed(failed) => {
            if let Some(TranscriptItem::Tool {
                status,
                output,
                event_id,
                ..
            }) = find_tool_mut(projection, Some(&failed.tool), None)
            {
                *status = "failed".to_owned();
                if output.is_none() {
                    *output = Some(failed.error);
                }
                *event_id = event.event_id.clone();
            } else {
                projection.transcript.push(TranscriptItem::Tool {
                    name: failed.tool,
                    status: "failed".to_owned(),
                    event_id: event.event_id.clone(),
                    arguments: None,
                    output: Some(failed.error),
                    call_id: None,
                });
            }
            touch_run(projection, event)?;
        }
        KnownPayload::ApprovalRequested(requested) => {
            projection.approvals.push(ApprovalView {
                id: requested.approval_id,
                summary: requested.summary.clone(),
                status: "pending".to_owned(),
            });
            projection.transcript.push(TranscriptItem::System {
                summary: format!("approval required: {}", requested.summary),
                event_id: event.event_id.clone(),
            });
        }
        KnownPayload::ApprovalResolved(resolved) => {
            if let Some(view) = projection
                .approvals
                .iter_mut()
                .find(|item| item.id == resolved.approval_id)
            {
                view.status = resolved.decision.clone();
            }
        }
        KnownPayload::UsageRecorded(usage) => {
            projection.input_tokens += usage.input_tokens;
            projection.output_tokens += usage.output_tokens;
        }
        KnownPayload::TaskCreated(task) => {
            projection.tasks.push(TaskView {
                id: task.task_id,
                title: task.title.clone(),
                status: if task.delay_until.is_some() {
                    TaskStatus::Scheduled
                } else {
                    TaskStatus::Queued
                },
            });
            projection.transcript.push(TranscriptItem::System {
                summary: format!("task created: {}", task.title),
                event_id: event.event_id.clone(),
            });
        }
        KnownPayload::TaskStarted(task) => {
            if let Some(view) = projection
                .tasks
                .iter_mut()
                .find(|item| item.id == task.task_id)
            {
                view.status = TaskStatus::Running;
            }
        }
        KnownPayload::TaskCompleted(task) => {
            if let Some(view) = projection
                .tasks
                .iter_mut()
                .find(|item| item.id == task.task_id)
            {
                view.status = TaskStatus::Completed;
            }
            projection.transcript.push(TranscriptItem::System {
                summary: format!(
                    "task completed: {}",
                    task.summary.unwrap_or_else(|| task.task_id.to_string())
                ),
                event_id: event.event_id.clone(),
            });
        }
        KnownPayload::TaskFailed(task) => {
            if let Some(view) = projection
                .tasks
                .iter_mut()
                .find(|item| item.id == task.task_id)
            {
                view.status = TaskStatus::Failed;
            }
            projection.transcript.push(TranscriptItem::System {
                summary: format!("task failed: {}", task.error),
                event_id: event.event_id.clone(),
            });
        }
        KnownPayload::TaskWakeup(task) => {
            projection.transcript.push(TranscriptItem::System {
                summary: format!("task wakeup {}", task.task_id),
                event_id: event.event_id.clone(),
            });
        }
        KnownPayload::SubagentSpawned(spawned) => {
            projection.subagents.push(SubagentView {
                id: spawned.agent_id,
                role: spawned.role.clone(),
                child_session_id: spawned.child_session_id,
                status: "running".to_owned(),
                summary: None,
            });
            projection.transcript.push(TranscriptItem::System {
                summary: format!("subagent spawned: {}", spawned.role),
                event_id: event.event_id.clone(),
            });
        }
        KnownPayload::SubagentCompleted(done) => {
            if let Some(view) = projection
                .subagents
                .iter_mut()
                .find(|item| item.id == done.agent_id)
            {
                view.status = "completed".to_owned();
                view.summary.clone_from(&done.summary);
            }
            projection.transcript.push(TranscriptItem::System {
                summary: format!(
                    "subagent completed: {}",
                    done.summary.unwrap_or_else(|| done.agent_id.to_string())
                ),
                event_id: event.event_id.clone(),
            });
        }
        KnownPayload::SubagentFailed(failed) => {
            if let Some(view) = projection
                .subagents
                .iter_mut()
                .find(|item| item.id == failed.agent_id)
            {
                view.status = "failed".to_owned();
                view.summary = Some(failed.error.clone());
            }
            projection.transcript.push(TranscriptItem::System {
                summary: format!("subagent failed: {}", failed.error),
                event_id: event.event_id.clone(),
            });
        }
        other => {
            apply_run_transition(projection, event, &other)?;
            if matches!(
                other,
                KnownPayload::ContextCompactionCompleted(_) | KnownPayload::CheckpointCreated(_)
            ) {
                projection.transcript.push(TranscriptItem::System {
                    summary: other.event_type().to_owned(),
                    event_id: event.event_id.clone(),
                });
            }
        }
    }

    if let Some(session) = projection.session.as_mut() {
        session.updated_at = event.timestamp;
    }
    Ok(())
}

fn compact_tool_args(value: &impl ToString) -> Option<String> {
    let raw = value.to_string();
    if raw == "null" || raw == "{}" || raw == "[]" || raw.is_empty() {
        None
    } else {
        Some(raw)
    }
}

fn find_tool_mut<'a>(
    projection: &'a mut SessionProjection,
    name: Option<&str>,
    call_id: Option<&str>,
) -> Option<&'a mut TranscriptItem> {
    projection.transcript.iter_mut().rev().find(|item| {
        let TranscriptItem::Tool {
            name: item_name,
            call_id: item_call,
            ..
        } = item
        else {
            return false;
        };
        if let Some(call_id) = call_id {
            return item_call.as_deref() == Some(call_id)
                || (item_call.is_none() && name.is_some_and(|name| item_name == name));
        }
        name.is_some_and(|name| item_name == name)
    })
}

fn apply_run_transition(
    projection: &mut SessionProjection,
    event: &EventEnvelope,
    payload: &KnownPayload,
) -> Result<()> {
    if event.run_id.is_none() {
        return Ok(());
    }
    let run = run_mut(projection, event)?;
    if let Some(next) = transition_run(run.status, payload)? {
        run.status = next;
        run.updated_at = event.timestamp;
        if let KnownPayload::RunStarted(started) = payload {
            if started.model.is_some() {
                run.model.clone_from(&started.model);
            }
            run.turn_id.clone_from(&event.turn_id);
        }
    }
    Ok(())
}

fn touch_run(projection: &mut SessionProjection, event: &EventEnvelope) -> Result<()> {
    if event.run_id.is_none() {
        return Ok(());
    }
    let run = run_mut(projection, event)?;
    run.updated_at = event.timestamp;
    Ok(())
}

fn touch_session(projection: &mut SessionProjection, timestamp: DateTime<Utc>) -> Result<()> {
    session_mut(projection)?.updated_at = timestamp;
    Ok(())
}

fn session_mut(projection: &mut SessionProjection) -> Result<&mut SessionRecord> {
    projection
        .session
        .as_mut()
        .ok_or_else(|| BloraError::event("session event before session.created"))
}

fn run_mut<'a>(
    projection: &'a mut SessionProjection,
    event: &EventEnvelope,
) -> Result<&'a mut RunRecord> {
    let run_id = event
        .run_id
        .as_ref()
        .ok_or_else(|| BloraError::event("run-scoped event missing run_id"))?;
    projection
        .runs
        .iter_mut()
        .find(|run| run.id == *run_id)
        .ok_or_else(|| BloraError::RunNotFound(run_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_events::{NewEvent, RunCreated, SessionCreated, UserInput};
    use blora_types::Mode;

    fn envelope(session: &SessionId, sequence: u64, payload: KnownPayload) -> EventEnvelope {
        EventEnvelope::stamped(NewEvent::new(session.clone(), payload), sequence).unwrap()
    }

    #[test]
    fn rebuilds_session_from_events() {
        let session_id = SessionId::generate();
        let created = envelope(
            &session_id,
            1,
            KnownPayload::SessionCreated(SessionCreated {
                title: Some("demo".to_owned()),
                workspace_path: "/tmp/ws".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            }),
        );
        let mut run_event = envelope(
            &session_id,
            2,
            KnownPayload::RunCreated(RunCreated {
                mode: Mode::Code,
                model: Some("mock".to_owned()),
            }),
        );
        let run_id = RunId::generate();
        run_event.run_id = Some(run_id.clone());
        let mut user = envelope(
            &session_id,
            3,
            KnownPayload::UserInput(UserInput {
                text: "hi".to_owned(),
            }),
        );
        user.run_id = Some(run_id);

        let projection = rebuild(&[created, run_event, user]).unwrap();
        assert_eq!(projection.last_sequence, 3);
        assert_eq!(projection.runs.len(), 1);
        assert!(matches!(
            projection.transcript[0],
            TranscriptItem::User { .. }
        ));
    }

    #[test]
    fn unknown_events_do_not_break_replay() {
        let session_id = SessionId::generate();
        let created = envelope(
            &session_id,
            1,
            KnownPayload::SessionCreated(SessionCreated {
                title: None,
                workspace_path: "/tmp/ws".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            }),
        );
        let mut unknown = created.clone();
        unknown.event_id = EventId::generate();
        unknown.sequence = 2;
        unknown.event_type = "experimental.future".to_owned();
        unknown.payload = serde_json::json!({"x": 1});
        let projection = rebuild(&[created, unknown]).unwrap();
        assert_eq!(projection.last_sequence, 2);
        assert!(projection.session.is_some());
    }
}
