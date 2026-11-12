// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{KnownPayload, TeamCreated, TeamTaskUpdated};
use blora_storage::{TeamRecord, TeamTaskRecord};
use blora_types::{AgentId, BloraError, Result, RunId, SessionId, TeamId, TeamTaskId, TurnId};
use chrono::Utc;
use serde_json::{Value, json};

use crate::coordinator::{RunOptions, Runtime};

impl Runtime {
    pub(crate) fn team_tool(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        arguments: &Value,
        options: &RunOptions,
    ) -> Result<String> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| BloraError::Other("team needs action".to_owned()))?;
        let team_id = arguments
            .get("team_id")
            .and_then(Value::as_str)
            .map(TeamId::parse)
            .transpose()?;
        match action {
            "create" => {
                let name = arguments
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
                    .ok_or_else(|| BloraError::Other("team create needs name".to_owned()))?;
                let workspace = self
                    .show_session(session_id)?
                    .session
                    .ok_or_else(|| BloraError::SessionNotFound(session_id.to_string()))?
                    .workspace_path;
                let id = TeamId::generate();
                let now = Utc::now();
                self.store.insert_team(&TeamRecord {
                    id: id.clone(),
                    workspace_path: workspace.clone(),
                    name: name.to_owned(),
                    description: arguments
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    created_at: now,
                    updated_at: now,
                })?;
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::TeamCreated(TeamCreated {
                        team_id: id.clone(),
                        name: name.to_owned(),
                    }),
                )?;
                Ok(format!("created team {id}"))
            }
            "list" => {
                let workspace = self
                    .show_session(session_id)?
                    .session
                    .ok_or_else(|| BloraError::SessionNotFound(session_id.to_string()))?
                    .workspace_path;
                let teams = self.store.list_teams(&workspace)?;
                if teams.is_empty() {
                    return Ok("(no teams)".to_owned());
                }
                Ok(teams
                    .iter()
                    .map(|team| format!("{}  {}", team.id, team.name))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "board" => {
                let team = self.require_team(team_id)?;
                let tasks = self.store.list_team_tasks(&team.id)?;
                if tasks.is_empty() {
                    return Ok(format!("team {} has no tasks", team.name));
                }
                Ok(tasks
                    .iter()
                    .map(format_team_task)
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "dispatch" => {
                let team = self.require_team(team_id)?;
                let tasks = self.store.list_team_tasks(&team.id)?;
                let limit = arguments
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map_or(3, |value| value.clamp(1, 8) as usize);
                let mut dispatched = Vec::new();
                for task in tasks
                    .iter()
                    .filter(|task| task.status == "ready")
                    .take(limit)
                {
                    let prompt = format!(
                        "Team task {}: {}\n\n{}",
                        task.id,
                        task.title,
                        task.description.as_deref().unwrap_or("")
                    );
                    let agent =
                        self.spawn_subagent_background(session_id, "worker", &prompt, options)?;
                    let agent_id = AgentId::parse(&agent)?;
                    self.store.set_team_task_status(
                        &task.id,
                        "in_progress",
                        Some(agent_id.as_str()),
                        None,
                        None,
                    )?;
                    self.emit(
                        session_id,
                        Some(run_id),
                        Some(turn_id),
                        KnownPayload::TeamTaskUpdated(TeamTaskUpdated {
                            task_id: task.id.clone(),
                            status: "in_progress".to_owned(),
                            action: "dispatch".to_owned(),
                        }),
                    )?;
                    dispatched.push(format!("{} -> {agent}", task.id));
                }
                if dispatched.is_empty() {
                    return Ok("no ready tasks to dispatch".to_owned());
                }
                Ok(dispatched.join("\n"))
            }
            "delete" => {
                let team = self.require_team(team_id)?;
                for task in self.store.list_team_tasks(&team.id)? {
                    self.store.delete_team_task(&task.id)?;
                }
                self.store.delete_team(&team.id)?;
                Ok(format!("deleted team {}", team.id))
            }
            other => Err(BloraError::Other(format!("unknown team action: {other}"))),
        }
    }

    pub(crate) fn team_task_tool(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        arguments: &Value,
    ) -> Result<String> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| BloraError::Other("team_task needs action".to_owned()))?;
        let task_id = arguments
            .get("task_id")
            .and_then(Value::as_str)
            .map(TeamTaskId::parse)
            .transpose()?;
        match action {
            "create" => {
                let team_id = arguments
                    .get("team_id")
                    .and_then(Value::as_str)
                    .map(TeamId::parse)
                    .transpose()?
                    .ok_or_else(|| {
                        BloraError::Other("team_task create needs team_id".to_owned())
                    })?;
                let title = arguments
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|title| !title.trim().is_empty())
                    .ok_or_else(|| BloraError::Other("team_task create needs title".to_owned()))?;
                let id = TeamTaskId::generate();
                let now = Utc::now();
                self.store.insert_team_task(&TeamTaskRecord {
                    id: id.clone(),
                    team_id,
                    title: title.to_owned(),
                    description: arguments
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    status: arguments
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("ready")
                        .to_owned(),
                    priority: arguments
                        .get("priority")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    assignee_agent_id: None,
                    depends_on_json: arguments.get("depends_on").map(Value::to_string),
                    blocked_reason: None,
                    review_state: None,
                    heartbeat_at: None,
                    created_at: now,
                    updated_at: now,
                    completed_at: None,
                })?;
                self.emit_task(session_id, run_id, turn_id, &id, "ready", "create")?;
                Ok(format!("created team task {id}"))
            }
            "get" => {
                let id = require_task(task_id)?;
                Ok(format_team_task(&self.require_team_task(&id)?))
            }
            "list" => {
                let team_id = arguments
                    .get("team_id")
                    .and_then(Value::as_str)
                    .map(TeamId::parse)
                    .transpose()?
                    .ok_or_else(|| BloraError::Other("team_task list needs team_id".to_owned()))?;
                let tasks = self.store.list_team_tasks(&team_id)?;
                if tasks.is_empty() {
                    return Ok("(no team tasks)".to_owned());
                }
                Ok(tasks
                    .iter()
                    .map(format_team_task)
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "claim" => {
                let id = require_task(task_id)?;
                let assignee = arguments.get("assignee").and_then(Value::as_str);
                self.store
                    .set_team_task_status(&id, "in_progress", assignee, None, None)?;
                self.emit_task(session_id, run_id, turn_id, &id, "in_progress", "claim")?;
                Ok(format!("claimed {id}"))
            }
            "complete" => {
                let id = require_task(task_id)?;
                self.store
                    .set_team_task_status(&id, "done", None, None, None)?;
                self.emit_task(session_id, run_id, turn_id, &id, "done", "complete")?;
                Ok(format!("completed {id}"))
            }
            "block" => {
                let id = require_task(task_id)?;
                let reason = arguments.get("reason").and_then(Value::as_str);
                self.store
                    .set_team_task_status(&id, "blocked", None, reason, None)?;
                self.emit_task(session_id, run_id, turn_id, &id, "blocked", "block")?;
                Ok(format!("blocked {id}"))
            }
            "unblock" => {
                let id = require_task(task_id)?;
                self.store
                    .set_team_task_status(&id, "ready", None, None, None)?;
                self.emit_task(session_id, run_id, turn_id, &id, "ready", "unblock")?;
                Ok(format!("unblocked {id}"))
            }
            "update" => {
                let id = require_task(task_id)?;
                let status = arguments
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("ready");
                let assignee = arguments.get("assignee").and_then(Value::as_str);
                let review = arguments.get("review_state").and_then(Value::as_str);
                self.store
                    .set_team_task_status(&id, status, assignee, None, review)?;
                self.emit_task(session_id, run_id, turn_id, &id, status, "update")?;
                Ok(format!("updated {id}"))
            }
            "heartbeat" => {
                let id = require_task(task_id)?;
                self.store.heartbeat_team_task(&id)?;
                Ok(format!("heartbeat {id}"))
            }
            "comment" => {
                let id = require_task(task_id)?;
                let body = arguments
                    .get("body")
                    .and_then(Value::as_str)
                    .filter(|body| !body.trim().is_empty())
                    .ok_or_else(|| BloraError::Other("team_task comment needs body".to_owned()))?;
                self.store.add_team_comment(&id, "agent", body)?;
                Ok(format!("commented on {id}"))
            }
            "delete" => {
                let id = require_task(task_id)?;
                self.store.delete_team_task(&id)?;
                self.emit_task(session_id, run_id, turn_id, &id, "deleted", "delete")?;
                Ok(format!("deleted {id}"))
            }
            other => Err(BloraError::Other(format!(
                "unknown team_task action: {other}"
            ))),
        }
    }

    fn require_team(&self, id: Option<TeamId>) -> Result<TeamRecord> {
        let id = id.ok_or_else(|| BloraError::Other("team action needs team_id".to_owned()))?;
        self.store
            .get_team(&id)?
            .ok_or_else(|| BloraError::Other("team not found".to_owned()))
    }

    fn require_team_task(&self, id: &TeamTaskId) -> Result<TeamTaskRecord> {
        self.store
            .get_team_task(id)?
            .ok_or_else(|| BloraError::Other("team task not found".to_owned()))
    }

    fn emit_task(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        task_id: &TeamTaskId,
        status: &str,
        action: &str,
    ) -> Result<()> {
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::TeamTaskUpdated(TeamTaskUpdated {
                task_id: task_id.clone(),
                status: status.to_owned(),
                action: action.to_owned(),
            }),
        )
    }
}

fn require_task(id: Option<TeamTaskId>) -> Result<TeamTaskId> {
    id.ok_or_else(|| BloraError::Other("team_task action needs task_id".to_owned()))
}

fn format_team_task(task: &TeamTaskRecord) -> String {
    json!({
        "id": task.id,
        "title": task.title,
        "status": task.status,
        "priority": task.priority,
        "assignee": task.assignee_agent_id,
        "description": task.description,
        "blocked_reason": task.blocked_reason,
        "review_state": task.review_state,
    })
    .to_string()
}
