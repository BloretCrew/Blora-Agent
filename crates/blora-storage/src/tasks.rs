// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_types::{AgentId, RunId, SessionId, TaskId, TaskStatus};
use chrono::{DateTime, Utc};

#[derive(Clone, Debug)]
pub struct CreateTask {
    pub session_id: SessionId,
    pub title: String,
    pub prompt: String,
    pub delay_until: Option<DateTime<Utc>>,
    pub max_attempts: u32,
    pub auto_approve: bool,
    pub mock: bool,
    pub cron: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TaskRecord {
    pub id: TaskId,
    pub session_id: SessionId,
    pub title: String,
    pub prompt: String,
    pub status: TaskStatus,
    pub run_id: Option<RunId>,
    pub delay_until: Option<DateTime<Utc>>,
    pub attempt: u32,
    pub max_attempts: u32,
    pub auto_approve: bool,
    pub mock: bool,
    pub cron: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct AgentRecord {
    pub id: AgentId,
    pub parent_session_id: SessionId,
    pub child_session_id: SessionId,
    pub role: String,
    pub depth: u32,
    pub status: String,
    pub budget_turns: u32,
    pub summary: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
