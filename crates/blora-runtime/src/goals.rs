// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{GoalUpdated, KnownPayload};
use blora_storage::GoalRecord;
use blora_types::{BloraError, GoalId, Result, RunId, SessionId, TurnId};
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::coordinator::Runtime;

impl Runtime {
    pub fn list_goals(&self, session_id: &SessionId) -> Result<Vec<GoalRecord>> {
        self.store.list_goals(session_id)
    }

    pub fn update_goal_from_api(
        &self,
        goal_id: &GoalId,
        status: Option<&str>,
        progress: Option<i64>,
        title: Option<&str>,
        priority: Option<i64>,
    ) -> Result<bool> {
        self.store
            .update_goal(goal_id, title, None, status, priority, progress, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn goal_tool(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        arguments: &Value,
    ) -> Result<String> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| BloraError::Other("goal needs action".to_owned()))?;
        let goal_id = arguments
            .get("goal_id")
            .and_then(Value::as_str)
            .map(GoalId::parse)
            .transpose()?;
        match action {
            "propose" => {
                let title = arguments
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|title| !title.trim().is_empty())
                    .ok_or_else(|| BloraError::Other("goal propose needs title".to_owned()))?;
                let id = GoalId::generate();
                let now = Utc::now();
                self.store.insert_goal(&GoalRecord {
                    id: id.clone(),
                    session_id: session_id.clone(),
                    parent_goal_id: None,
                    title: title.to_owned(),
                    description: arguments
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    status: "proposed".to_owned(),
                    priority: arguments
                        .get("priority")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    progress: 0,
                    due_at: parse_time(arguments.get("due_at").and_then(Value::as_str))?,
                    created_at: now,
                    updated_at: now,
                })?;
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::GoalUpdated(GoalUpdated {
                        goal_id: id.clone(),
                        status: "proposed".to_owned(),
                        action: "propose".to_owned(),
                    }),
                )?;
                Ok(format!("proposed goal {id}"))
            }
            "update" | "approve" | "reject" => {
                let id = goal_id
                    .ok_or_else(|| BloraError::Other("goal action needs goal_id".to_owned()))?;
                let status = match action {
                    "approve" => Some("active"),
                    "reject" => Some("rejected"),
                    _ => arguments.get("status").and_then(Value::as_str),
                };
                let changed = self.store.update_goal(
                    &id,
                    arguments.get("title").and_then(Value::as_str),
                    arguments.get("description").and_then(Value::as_str),
                    status,
                    arguments.get("priority").and_then(Value::as_i64),
                    arguments.get("progress").and_then(Value::as_i64),
                    parse_time(arguments.get("due_at").and_then(Value::as_str))?,
                )?;
                if !changed {
                    return Err(BloraError::Other("goal not found".to_owned()));
                }
                let status = status.unwrap_or("updated").to_owned();
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::GoalUpdated(GoalUpdated {
                        goal_id: id.clone(),
                        status: status.clone(),
                        action: action.to_owned(),
                    }),
                )?;
                Ok(format!("goal {id} {status}"))
            }
            "get" => {
                let id = goal_id
                    .ok_or_else(|| BloraError::Other("goal get needs goal_id".to_owned()))?;
                let goal = self
                    .store
                    .get_goal(&id)?
                    .ok_or_else(|| BloraError::Other("goal not found".to_owned()))?;
                Ok(format_goal(&goal))
            }
            "list" => {
                let goals = self.store.list_goals(session_id)?;
                if goals.is_empty() {
                    return Ok("(no goals)".to_owned());
                }
                Ok(goals.iter().map(format_goal).collect::<Vec<_>>().join("\n"))
            }
            other => Err(BloraError::Other(format!("unknown goal action: {other}"))),
        }
    }
}

fn parse_time(value: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|time| time.with_timezone(&Utc))
                .map_err(|err| BloraError::Other(format!("invalid due_at: {err}")))
        })
        .transpose()
}

fn format_goal(goal: &GoalRecord) -> String {
    format!(
        "{}  {}  {}%  priority {}  {}{}",
        goal.id,
        goal.status,
        goal.progress,
        goal.priority,
        goal.title,
        goal.description
            .as_deref()
            .map(|description| format!("  {description}"))
            .unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_storage::{CreateSession, SqliteStore};

    #[test]
    fn api_update_goal_changes_progress_and_status() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session_id = runtime
            .create_session(CreateSession {
                title: Some("t".to_owned()),
                workspace_path: "/tmp".to_owned(),
                mode: blora_types::Mode::Work,
                parent_session_id: None,
            })
            .unwrap();
        let now = Utc::now();
        let goal_id = GoalId::generate();
        runtime
            .store
            .insert_goal(&GoalRecord {
                id: goal_id.clone(),
                session_id,
                parent_goal_id: None,
                title: "Ship".to_owned(),
                description: None,
                status: "proposed".to_owned(),
                priority: 0,
                progress: 0,
                due_at: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
        assert!(
            runtime
                .update_goal_from_api(&goal_id, Some("active"), Some(40), None, None)
                .unwrap()
        );
        let goal = runtime.store.get_goal(&goal_id).unwrap().unwrap();
        assert_eq!(goal.status, "active");
        assert_eq!(goal.progress, 40);
    }
}
