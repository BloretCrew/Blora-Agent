// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{KnownPayload, TeamCreated, TeamTaskUpdated};
use blora_storage::{TeamColumnRecord, TeamRecord, TeamTaskRecord};
use blora_types::{AgentId, BloraError, Result, RunId, SessionId, TeamId, TeamTaskId, TurnId};
use chrono::Utc;
use serde_json::{Value, json};

use crate::coordinator::{RunOptions, Runtime};

impl Runtime {
    pub fn list_teams(&self, workspace_path: &str) -> Result<Vec<TeamRecord>> {
        self.store.list_teams(workspace_path)
    }

    pub fn team_board(
        &self,
        team_id: &TeamId,
    ) -> Result<(Vec<TeamColumnRecord>, Vec<TeamTaskRecord>)> {
        Ok((
            self.store.list_team_columns(team_id)?,
            self.store.list_team_tasks(team_id)?,
        ))
    }

    pub fn move_team_task(
        &self,
        team_id: &TeamId,
        task_id: &TeamTaskId,
        column_id: &str,
    ) -> Result<TeamTaskRecord> {
        let task = self.require_team_task(task_id)?;
        if &task.team_id != team_id {
            return Err(BloraError::Other(format!(
                "task {task_id} does not belong to team {team_id}"
            )));
        }
        let column = self
            .store
            .list_team_columns(team_id)?
            .into_iter()
            .find(|column| column.id == column_id)
            .ok_or_else(|| BloraError::Other(format!("unknown column: {column_id}")))?;
        let status = column.name.to_ascii_lowercase().replace(' ', "_");
        self.store
            .set_team_task_status(task_id, &status, Some(column_id), None, None, None)?;
        self.require_team_task(task_id)
    }

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
                for (position, name) in [
                    "Backlog",
                    "Ready",
                    "In Progress",
                    "Review",
                    "Blocked",
                    "Done",
                ]
                .iter()
                .enumerate()
                {
                    self.store.insert_team_column(&TeamColumnRecord {
                        id: format!("col_{}_{position}", id.as_str()),
                        team_id: id.clone(),
                        name: (*name).to_owned(),
                        position: position as i64,
                        wip_limit: None,
                    })?;
                }
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
                let columns = self.store.list_team_columns(&team.id)?;
                if tasks.is_empty() {
                    return Ok(format!("team {} has no tasks", team.name));
                }
                if columns.is_empty() {
                    return Ok(tasks
                        .iter()
                        .map(format_team_task)
                        .collect::<Vec<_>>()
                        .join("\n"));
                }
                Ok(columns
                    .iter()
                    .map(|column| {
                        let items = tasks
                            .iter()
                            .filter(|task| {
                                task.column_id.as_deref() == Some(column.id.as_str())
                                    || (task.column_id.is_none()
                                        && status_matches_column(&task.status, &column.name))
                            })
                            .map(format_team_task)
                            .collect::<Vec<_>>();
                        format!("## {}\n{}", column.name, items.join("\n"))
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"))
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
                    let column = self.column_id_for(&task.team_id, "in_progress");
                    self.store.set_team_task_status(
                        &task.id,
                        "in_progress",
                        column.as_deref(),
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
                let status = arguments
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("ready")
                    .to_owned();
                let column_id = self.column_id_for(&team_id, &status);
                self.store.insert_team_task(&TeamTaskRecord {
                    id: id.clone(),
                    team_id,
                    column_id,
                    title: title.to_owned(),
                    description: arguments
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    status: status.clone(),
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
                self.emit_task(session_id, run_id, turn_id, &id, &status, "create")?;
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
                let task = self.require_team_task(&id)?;
                self.ensure_dependencies_done(&task)?;
                let assignee = arguments.get("assignee").and_then(Value::as_str);
                let column = self.column_id_for(&task.team_id, "in_progress");
                self.store.set_team_task_status(
                    &id,
                    "in_progress",
                    column.as_deref(),
                    assignee,
                    None,
                    None,
                )?;
                self.emit_task(session_id, run_id, turn_id, &id, "in_progress", "claim")?;
                Ok(format!("claimed {id}"))
            }
            "complete" => {
                let id = require_task(task_id)?;
                let task = self.require_team_task(&id)?;
                let column = self.column_id_for(&task.team_id, "done");
                self.store.set_team_task_status(
                    &id,
                    "done",
                    column.as_deref(),
                    None,
                    None,
                    None,
                )?;
                self.emit_task(session_id, run_id, turn_id, &id, "done", "complete")?;
                Ok(format!("completed {id}"))
            }
            "block" => {
                let id = require_task(task_id)?;
                let task = self.require_team_task(&id)?;
                let reason = arguments.get("reason").and_then(Value::as_str);
                let column = self.column_id_for(&task.team_id, "blocked");
                self.store.set_team_task_status(
                    &id,
                    "blocked",
                    column.as_deref(),
                    None,
                    reason,
                    None,
                )?;
                self.emit_task(session_id, run_id, turn_id, &id, "blocked", "block")?;
                Ok(format!("blocked {id}"))
            }
            "unblock" => {
                let id = require_task(task_id)?;
                let task = self.require_team_task(&id)?;
                let column = self.column_id_for(&task.team_id, "ready");
                self.store.set_team_task_status(
                    &id,
                    "ready",
                    column.as_deref(),
                    None,
                    None,
                    None,
                )?;
                self.emit_task(session_id, run_id, turn_id, &id, "ready", "unblock")?;
                Ok(format!("unblocked {id}"))
            }
            "request_review" | "request_changes" | "approve" => {
                let id = require_task(task_id)?;
                let task = self.require_team_task(&id)?;
                let (status, review) = match action {
                    "request_review" => ("review", "requested"),
                    "request_changes" => ("in_progress", "changes_requested"),
                    _ => ("done", "approved"),
                };
                let column = self.column_id_for(&task.team_id, status);
                self.store.set_team_task_status(
                    &id,
                    status,
                    column.as_deref(),
                    None,
                    None,
                    Some(review),
                )?;
                self.emit_task(session_id, run_id, turn_id, &id, status, action)?;
                Ok(format!("{action} {id}"))
            }
            "update" => {
                let id = require_task(task_id)?;
                let task = self.require_team_task(&id)?;
                let status = arguments
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("ready");
                let assignee = arguments.get("assignee").and_then(Value::as_str);
                let review = arguments.get("review_state").and_then(Value::as_str);
                let column = self.column_id_for(&task.team_id, status);
                self.store.set_team_task_status(
                    &id,
                    status,
                    column.as_deref(),
                    assignee,
                    None,
                    review,
                )?;
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
            "comments" => {
                let id = require_task(task_id)?;
                let comments = self.store.list_team_comments(&id)?;
                if comments.is_empty() {
                    return Ok("(no comments)".to_owned());
                }
                Ok(comments
                    .iter()
                    .map(|(author, body, at)| format!("{author} {}: {body}", at.to_rfc3339()))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "attach" => {
                let id = require_task(task_id)?;
                let path = arguments
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| BloraError::Other("team_task attach needs path".to_owned()))?;
                let name = arguments
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(path);
                let mime = arguments
                    .get("mime")
                    .and_then(Value::as_str)
                    .unwrap_or("application/octet-stream");
                let size = arguments.get("size").and_then(Value::as_i64).unwrap_or(0);
                self.store
                    .add_team_attachment(&id, name, mime, size, "", path)?;
                Ok(format!("attached {path} to {id}"))
            }
            "attachments" => {
                let id = require_task(task_id)?;
                let attachments = self.store.list_team_attachments(&id)?;
                if attachments.is_empty() {
                    return Ok("(no attachments)".to_owned());
                }
                Ok(attachments
                    .iter()
                    .map(|attachment| {
                        format!(
                            "{}  {}  {}  {}",
                            attachment.name, attachment.mime, attachment.size, attachment.path
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "link" => {
                let id = require_task(task_id)?;
                let kind = arguments
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("url");
                let target = arguments
                    .get("target")
                    .and_then(Value::as_str)
                    .ok_or_else(|| BloraError::Other("team_task link needs target".to_owned()))?;
                self.store.add_team_link(&id, kind, target)?;
                Ok(format!("linked {target} to {id}"))
            }
            "links" => {
                let id = require_task(task_id)?;
                let links = self.store.list_team_links(&id)?;
                if links.is_empty() {
                    return Ok("(no links)".to_owned());
                }
                Ok(links
                    .iter()
                    .map(|link| format!("{}  {}", link.kind, link.target))
                    .collect::<Vec<_>>()
                    .join("\n"))
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

    fn column_id_for(&self, team_id: &TeamId, status: &str) -> Option<String> {
        self.store
            .list_team_columns(team_id)
            .ok()?
            .into_iter()
            .find(|column| status_matches_column(status, &column.name))
            .map(|column| column.id)
    }

    fn ensure_dependencies_done(&self, task: &TeamTaskRecord) -> Result<()> {
        let Some(dependencies) = &task.depends_on_json else {
            return Ok(());
        };
        let ids: Vec<String> = serde_json::from_str(dependencies).unwrap_or_default();
        for id in ids {
            let dependency = TeamTaskId::parse(&id)?;
            if let Some(dependency) = self.store.get_team_task(&dependency)? {
                if dependency.status != "done" {
                    return Err(BloraError::Other(format!(
                        "task {} depends on {} which is {}",
                        task.id, dependency.id, dependency.status
                    )));
                }
            }
        }
        Ok(())
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

fn status_matches_column(status: &str, name: &str) -> bool {
    name.to_ascii_lowercase().replace(' ', "_") == status
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

#[cfg(test)]
mod tests {
    use super::*;
    use blora_storage::SqliteStore;

    #[test]
    fn move_team_task_updates_column_and_status() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let team_id = TeamId::generate();
        let now = Utc::now();
        runtime
            .store
            .insert_team(&TeamRecord {
                id: team_id.clone(),
                workspace_path: "/tmp".to_owned(),
                name: "Board".to_owned(),
                description: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
        runtime
            .store
            .insert_team_column(&TeamColumnRecord {
                id: "col_progress".to_owned(),
                team_id: team_id.clone(),
                name: "In Progress".to_owned(),
                position: 1,
                wip_limit: None,
            })
            .unwrap();
        let task_id = TeamTaskId::generate();
        runtime
            .store
            .insert_team_task(&TeamTaskRecord {
                id: task_id.clone(),
                team_id: team_id.clone(),
                column_id: None,
                title: "Ship it".to_owned(),
                description: None,
                status: "backlog".to_owned(),
                priority: 1,
                assignee_agent_id: None,
                depends_on_json: None,
                blocked_reason: None,
                review_state: None,
                heartbeat_at: None,
                created_at: now,
                updated_at: now,
                completed_at: None,
            })
            .unwrap();
        let moved = runtime
            .move_team_task(&team_id, &task_id, "col_progress")
            .unwrap();
        assert_eq!(moved.column_id.as_deref(), Some("col_progress"));
        assert_eq!(moved.status, "in_progress");
    }

    #[test]
    fn move_team_task_rejects_foreign_team() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let team_id = TeamId::generate();
        let other = TeamId::generate();
        let now = Utc::now();
        runtime
            .store
            .insert_team(&TeamRecord {
                id: other.clone(),
                workspace_path: "/tmp".to_owned(),
                name: "Other".to_owned(),
                description: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
        let task_id = TeamTaskId::generate();
        runtime
            .store
            .insert_team_task(&TeamTaskRecord {
                id: task_id.clone(),
                team_id: other,
                column_id: None,
                title: "Foreign".to_owned(),
                description: None,
                status: "backlog".to_owned(),
                priority: 0,
                assignee_agent_id: None,
                depends_on_json: None,
                blocked_reason: None,
                review_state: None,
                heartbeat_at: None,
                created_at: now,
                updated_at: now,
                completed_at: None,
            })
            .unwrap();
        assert!(
            runtime
                .move_team_task(&team_id, &task_id, "col_progress")
                .is_err()
        );
    }
}
