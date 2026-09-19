// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::{AgentRecord, CreateTask, TaskRecord};
use blora_events::{EventEnvelope, KnownPayload, NewEvent, SessionCreated};
use blora_session::{SessionProjection, rebuild};
use blora_types::{
    Actor, AgentId, ArtifactId, BloraError, Clock, EventId, Mode, Result, RunId, SCHEMA_VERSION,
    SessionId, SessionStatus, SystemClock, TaskId, TaskStatus, TurnId, Visibility,
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

const SCHEMA_SQL: &str = include_str!("schema.sql");

#[derive(Clone, Debug)]
pub struct CreateSession {
    pub title: Option<String>,
    pub workspace_path: String,
    pub mode: Mode,
    pub parent_session_id: Option<SessionId>,
}

#[derive(Clone, Debug)]
pub struct MemoryRecord {
    pub workspace_path: String,
    pub key: String,
    pub value: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ArtifactRecord {
    pub id: ArtifactId,
    pub session_id: SessionId,
    pub kind: String,
    pub path: Option<String>,
    pub content: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default)]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
}

#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub id: SessionId,
    pub title: Option<String>,
    pub workspace_path: String,
    pub mode: Mode,
    pub status: SessionStatus,
    pub updated_at: DateTime<Utc>,
    pub last_sequence: u64,
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
    clock: SystemClock,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(BloraError::storage)?;
            restrict_home(parent);
        }
        let conn = Connection::open(path).map_err(BloraError::storage)?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(BloraError::storage)?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )
        .map_err(BloraError::storage)?;
        conn.execute_batch(SCHEMA_SQL)
            .map_err(BloraError::storage)?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (1, ?1)",
            params![now],
        )
        .map_err(BloraError::storage)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (2, ?1)",
            params![now],
        )
        .map_err(BloraError::storage)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (3, ?1)",
            params![now],
        )
        .map_err(BloraError::storage)?;
        let _ = conn.execute("ALTER TABLE tasks ADD COLUMN cron TEXT", []);
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (4, ?1)",
            params![now],
        )
        .map_err(BloraError::storage)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (5, ?1)",
            params![now],
        )
        .map_err(BloraError::storage)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (6, ?1)",
            params![now],
        )
        .map_err(BloraError::storage)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (7, ?1)",
            params![now],
        )
        .map_err(BloraError::storage)?;
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN user_id TEXT", []);
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (8, ?1)",
            params![now],
        )
        .map_err(BloraError::storage)?;
        for statement in [
            "ALTER TABLE users ADD COLUMN passport_username TEXT",
            "ALTER TABLE users ADD COLUMN passport_nickname TEXT",
            "ALTER TABLE users ADD COLUMN passport_avatar TEXT",
            "ALTER TABLE users ADD COLUMN passport_email TEXT",
            "ALTER TABLE users ADD COLUMN passport_app_token TEXT",
            "ALTER TABLE users ADD COLUMN passport_refresh_token TEXT",
            "ALTER TABLE users ADD COLUMN passport_token_expires_at TEXT",
        ] {
            let _ = conn.execute(statement, []);
        }
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_users_passport_username ON users(passport_username)",
            [],
        )
        .map_err(BloraError::storage)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (9, ?1)",
            params![now],
        )
        .map_err(BloraError::storage)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (10, ?1)",
            params![now],
        )
        .map_err(BloraError::storage)?;
        Ok(Self {
            conn: Mutex::new(conn),
            clock: SystemClock,
        })
    }

    /// Current storage schema version written by this build.
    pub const STORE_VERSION: u32 = 10;

    pub fn queue_steer(&self, session_id: &SessionId, message: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO steers (session_id, message, created_at) VALUES (?1, ?2, ?3)",
            params![session_id.as_str(), message, Utc::now().to_rfc3339()],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }

    /// Pop every unconsumed steering message for the session, oldest first.
    pub fn take_steers(&self, session_id: &SessionId) -> Result<Vec<String>> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(BloraError::storage)?;
        let pending: Vec<(i64, String)> = {
            let mut stmt = tx
                .prepare(
                    "SELECT id, message FROM steers WHERE session_id = ?1 AND consumed_at IS NULL ORDER BY id ASC",
                )
                .map_err(BloraError::storage)?;
            let rows = stmt
                .query_map(params![session_id.as_str()], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(BloraError::storage)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(BloraError::storage)?);
            }
            out
        };
        let now = Utc::now().to_rfc3339();
        for (id, _) in &pending {
            tx.execute(
                "UPDATE steers SET consumed_at = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(BloraError::storage)?;
        }
        tx.commit().map_err(BloraError::storage)?;
        Ok(pending.into_iter().map(|(_, message)| message).collect())
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn create_session(&self, spec: CreateSession) -> Result<SessionId> {
        let session_id = SessionId::generate();
        let now = self.clock.now();
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(BloraError::storage)?;
        tx.execute(
            "INSERT INTO sessions (id, title, workspace_path, mode, status, parent_session_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id.as_str(),
                spec.title,
                spec.workspace_path,
                spec.mode.as_str(),
                SessionStatus::Active.as_str(),
                spec.parent_session_id.as_ref().map(SessionId::as_str),
                now.to_rfc3339(),
                now.to_rfc3339(),
            ],
        )
        .map_err(BloraError::storage)?;

        let envelope = EventEnvelope::from_new(
            NewEvent::new(
                session_id.clone(),
                KnownPayload::SessionCreated(SessionCreated {
                    title: spec.title.clone(),
                    workspace_path: spec.workspace_path.clone(),
                    mode: spec.mode,
                    parent_session_id: spec.parent_session_id.clone(),
                }),
            ),
            1,
            &self.clock,
        )?;
        insert_event(&tx, &envelope)?;
        tx.execute(
            "INSERT INTO workspaces (path, last_session_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(path) DO UPDATE SET last_session_id = excluded.last_session_id, updated_at = excluded.updated_at",
            params![spec.workspace_path, session_id.as_str(), now.to_rfc3339()],
        )
        .map_err(BloraError::storage)?;
        tx.commit().map_err(BloraError::storage)?;
        Ok(session_id)
    }

    pub fn append(&self, new: NewEvent) -> Result<EventEnvelope> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(BloraError::storage)?;
        session_exists(&tx, &new.session_id)?;
        let next = next_sequence(&tx, &new.session_id)?;
        let envelope = EventEnvelope::from_new(new, next, &self.clock)?;
        envelope.validate()?;
        insert_event(&tx, &envelope)?;
        apply_denormalized(&tx, &envelope)?;
        tx.commit().map_err(BloraError::storage)?;
        Ok(envelope)
    }

    pub fn load_events(&self, session_id: &SessionId) -> Result<Vec<EventEnvelope>> {
        self.load_events_after(session_id, 0)
    }

    /// Events with `sequence > after`, in order. `after = 0` loads everything.
    pub fn load_events_after(
        &self,
        session_id: &SessionId,
        after: u64,
    ) -> Result<Vec<EventEnvelope>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT event_id, schema_version, session_id, run_id, turn_id, parent_event_id,
                        causation_id, sequence, timestamp, actor, visibility, event_type, payload, metadata
                 FROM events WHERE session_id = ?1 AND sequence > ?2 ORDER BY sequence ASC",
            )
            .map_err(BloraError::storage)?;
        let rows = stmt
            .query_map(params![session_id.as_str(), after as i64], |row| {
                Ok(EventRow {
                    event_id: row.get(0)?,
                    schema_version: row.get(1)?,
                    session_id: row.get(2)?,
                    run_id: row.get(3)?,
                    turn_id: row.get(4)?,
                    parent_event_id: row.get(5)?,
                    causation_id: row.get(6)?,
                    sequence: row.get(7)?,
                    timestamp: row.get(8)?,
                    actor: row.get(9)?,
                    visibility: row.get(10)?,
                    event_type: row.get(11)?,
                    payload: row.get(12)?,
                    metadata: row.get(13)?,
                })
            })
            .map_err(BloraError::storage)?;
        let mut events = Vec::new();
        for row in rows {
            let row = row.map_err(BloraError::storage)?;
            events.push(row.into_envelope()?);
        }
        Ok(events)
    }

    pub fn load_projection(&self, session_id: &SessionId) -> Result<SessionProjection> {
        let events = self.load_events(session_id)?;
        if events.is_empty() {
            return Err(BloraError::SessionNotFound(session_id.to_string()));
        }
        rebuild(&events)
    }

    pub fn last_sequence(&self, session_id: &SessionId) -> Result<u64> {
        let conn = self.lock();
        let max: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| row.get(0),
            )
            .map_err(BloraError::storage)?;
        Ok(u64::try_from(max).unwrap_or(0))
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.title, s.workspace_path, s.mode, s.status, s.updated_at,
                        COALESCE((SELECT MAX(sequence) FROM events e WHERE e.session_id = s.id), 0)
                 FROM sessions s
                 ORDER BY s.updated_at DESC",
            )
            .map_err(BloraError::storage)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })
            .map_err(BloraError::storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, title, workspace_path, mode, status, updated_at, last_sequence) =
                row.map_err(BloraError::storage)?;
            out.push(SessionSummary {
                id: SessionId::parse(&id)?,
                title,
                workspace_path,
                mode: Mode::parse(&mode)?,
                status: SessionStatus::parse(&status)?,
                updated_at: parse_time(&updated_at)?,
                last_sequence: u64::try_from(last_sequence).unwrap_or(0),
            });
        }
        Ok(out)
    }

    pub fn session_parent(&self, session_id: &SessionId) -> Result<Option<SessionId>> {
        let conn = self.lock();
        let parent: Option<Option<String>> = conn
            .query_row(
                "SELECT parent_session_id FROM sessions WHERE id = ?1",
                params![session_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(BloraError::storage)?;
        match parent.flatten() {
            Some(id) => Ok(Some(SessionId::parse(&id)?)),
            None => Ok(None),
        }
    }

    pub fn insert_task(&self, id: &TaskId, spec: &CreateTask, now: DateTime<Utc>) -> Result<()> {
        let status = if spec.delay_until.is_some() {
            TaskStatus::Scheduled
        } else {
            TaskStatus::Queued
        };
        let conn = self.lock();
        conn.execute(
            "INSERT INTO tasks (
                id, session_id, title, prompt, status, run_id, delay_until,
                attempt, max_attempts, auto_approve, mock, cron, error, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, 0, ?7, ?8, ?9, ?10, NULL, ?11, ?12)",
            params![
                id.as_str(),
                spec.session_id.as_str(),
                spec.title,
                spec.prompt,
                status.as_str(),
                spec.delay_until.map(|t| t.to_rfc3339()),
                spec.max_attempts as i64,
                i64::from(spec.auto_approve),
                i64::from(spec.mock),
                spec.cron,
                now.to_rfc3339(),
                now.to_rfc3339(),
            ],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }

    pub fn get_task(&self, task_id: &TaskId) -> Result<TaskRecord> {
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT id, session_id, title, prompt, status, run_id, delay_until,
                        attempt, max_attempts, auto_approve, mock, error, created_at, updated_at, cron
                 FROM tasks WHERE id = ?1",
                params![task_id.as_str()],
                task_row,
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => {
                    BloraError::Other(format!("task not found: {task_id}"))
                }
                other => BloraError::storage(other),
            })?;
        row_to_task(row)
    }

    pub fn list_tasks(&self, session_id: Option<&SessionId>) -> Result<Vec<TaskRecord>> {
        let conn = self.lock();
        let mut out = Vec::new();
        if let Some(session_id) = session_id {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, title, prompt, status, run_id, delay_until,
                            attempt, max_attempts, auto_approve, mock, error, created_at, updated_at, cron
                     FROM tasks WHERE session_id = ?1 ORDER BY created_at DESC",
                )
                .map_err(BloraError::storage)?;
            let rows = stmt
                .query_map(params![session_id.as_str()], task_row)
                .map_err(BloraError::storage)?;
            for row in rows {
                out.push(row_to_task(row.map_err(BloraError::storage)?)?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, title, prompt, status, run_id, delay_until,
                            attempt, max_attempts, auto_approve, mock, error, created_at, updated_at, cron
                     FROM tasks ORDER BY created_at DESC",
                )
                .map_err(BloraError::storage)?;
            let rows = stmt.query_map([], task_row).map_err(BloraError::storage)?;
            for row in rows {
                out.push(row_to_task(row.map_err(BloraError::storage)?)?);
            }
        }
        Ok(out)
    }

    pub fn claim_due_tasks(&self, now: DateTime<Utc>) -> Result<Vec<TaskRecord>> {
        let due = self.list_due(now)?;
        let mut claimed = Vec::new();
        for task in due {
            if self.cas_task_status(
                &task.id,
                &[TaskStatus::Queued, TaskStatus::Scheduled],
                TaskStatus::Running,
                now,
                None,
                None,
            )? {
                let mut running = self.get_task(&task.id)?;
                running.attempt += 1;
                self.set_task_attempt(&running.id, running.attempt, now)?;
                claimed.push(self.get_task(&task.id)?);
            }
        }
        Ok(claimed)
    }

    fn list_due(&self, now: DateTime<Utc>) -> Result<Vec<TaskRecord>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, title, prompt, status, run_id, delay_until,
                        attempt, max_attempts, auto_approve, mock, error, created_at, updated_at, cron
                 FROM tasks
                 WHERE status IN ('queued', 'scheduled')
                   AND (delay_until IS NULL OR delay_until <= ?1)
                 ORDER BY created_at ASC",
            )
            .map_err(BloraError::storage)?;
        let rows = stmt
            .query_map(params![now.to_rfc3339()], task_row)
            .map_err(BloraError::storage)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row_to_task(row.map_err(BloraError::storage)?)?);
        }
        Ok(out)
    }

    pub fn cas_task_status(
        &self,
        task_id: &TaskId,
        from: &[TaskStatus],
        to: TaskStatus,
        now: DateTime<Utc>,
        error: Option<&str>,
        run_id: Option<&RunId>,
    ) -> Result<bool> {
        let placeholders = from
            .iter()
            .map(|status| format!("'{}'", status.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "UPDATE tasks SET status = ?1, updated_at = ?2, error = ?3, run_id = COALESCE(?4, run_id)
             WHERE id = ?5 AND status IN ({placeholders})"
        );
        let conn = self.lock();
        let changed = conn
            .execute(
                &sql,
                params![
                    to.as_str(),
                    now.to_rfc3339(),
                    error,
                    run_id.map(RunId::as_str),
                    task_id.as_str(),
                ],
            )
            .map_err(BloraError::storage)?;
        Ok(changed > 0)
    }

    pub fn set_task_attempt(
        &self,
        task_id: &TaskId,
        attempt: u32,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE tasks SET attempt = ?1, updated_at = ?2 WHERE id = ?3",
            params![attempt as i64, now.to_rfc3339(), task_id.as_str()],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }

    pub fn reschedule_task(
        &self,
        task_id: &TaskId,
        delay_until: DateTime<Utc>,
        now: DateTime<Utc>,
        error: &str,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE tasks SET status = 'scheduled', delay_until = ?1, updated_at = ?2, error = ?3
             WHERE id = ?4",
            params![
                delay_until.to_rfc3339(),
                now.to_rfc3339(),
                error,
                task_id.as_str()
            ],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }

    pub fn insert_agent(&self, record: &AgentRecord) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO agents (
                id, parent_session_id, child_session_id, role, depth, status,
                budget_turns, summary, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.id.as_str(),
                record.parent_session_id.as_str(),
                record.child_session_id.as_str(),
                record.role,
                record.depth as i64,
                record.status,
                record.budget_turns as i64,
                record.summary,
                record.created_at.to_rfc3339(),
                record.updated_at.to_rfc3339(),
            ],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }

    pub fn update_agent(
        &self,
        agent_id: &AgentId,
        status: &str,
        summary: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE agents SET status = ?1, summary = ?2, updated_at = ?3 WHERE id = ?4",
            params![status, summary, now.to_rfc3339(), agent_id.as_str()],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }

    pub fn running_child_count(&self, parent: &SessionId) -> Result<u32> {
        let conn = self.lock();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agents WHERE parent_session_id = ?1 AND status = 'running'",
                params![parent.as_str()],
                |row| row.get(0),
            )
            .map_err(BloraError::storage)?;
        Ok(u32::try_from(count).unwrap_or(0))
    }

    pub fn get_agent(&self, agent_id: &AgentId) -> Result<AgentRecord> {
        let agents = self.list_agents_filtered(Some(agent_id), None)?;
        agents
            .into_iter()
            .next()
            .ok_or_else(|| BloraError::Other(format!("agent not found: {agent_id}")))
    }

    pub fn archive_session(&self, session_id: &SessionId, now: DateTime<Utc>) -> Result<()> {
        let conn = self.lock();
        let changed = conn
            .execute(
                "UPDATE sessions SET status = 'archived', updated_at = ?1 WHERE id = ?2",
                params![now.to_rfc3339(), session_id.as_str()],
            )
            .map_err(BloraError::storage)?;
        if changed == 0 {
            return Err(BloraError::SessionNotFound(session_id.to_string()));
        }
        Ok(())
    }

    pub fn upsert_memory(
        &self,
        workspace_path: &str,
        key: &str,
        value: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO memories (workspace_path, key, value, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(workspace_path, key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![workspace_path, key, value, now.to_rfc3339()],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }

    pub fn list_memories(&self, workspace_path: &str) -> Result<Vec<MemoryRecord>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT workspace_path, key, value, updated_at FROM memories
                 WHERE workspace_path = ?1 ORDER BY key ASC",
            )
            .map_err(BloraError::storage)?;
        let rows = stmt
            .query_map(params![workspace_path], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(BloraError::storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (workspace_path, key, value, updated_at) = row.map_err(BloraError::storage)?;
            out.push(MemoryRecord {
                workspace_path,
                key,
                value,
                updated_at: parse_time(&updated_at)?,
            });
        }
        Ok(out)
    }

    pub fn get_memory(&self, workspace_path: &str, key: &str) -> Result<Option<MemoryRecord>> {
        Ok(self
            .list_memories(workspace_path)?
            .into_iter()
            .find(|item| item.key == key))
    }

    pub fn delete_memory(&self, workspace_path: &str, key: &str) -> Result<bool> {
        let conn = self.lock();
        let changed = conn
            .execute(
                "DELETE FROM memories WHERE workspace_path = ?1 AND key = ?2",
                params![workspace_path, key],
            )
            .map_err(BloraError::storage)?;
        Ok(changed > 0)
    }

    pub fn usage_totals(&self, session_id: Option<&SessionId>) -> Result<UsageTotals> {
        let conn = self.lock();
        let (input, output, cached): (i64, i64, i64) = if let Some(session_id) = session_id {
            conn.query_row(
                "SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0), COALESCE(SUM(cached_tokens),0)
                 FROM provider_usage WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(BloraError::storage)?
        } else {
            conn.query_row(
                "SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0), COALESCE(SUM(cached_tokens),0)
                 FROM provider_usage",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(BloraError::storage)?
        };
        Ok(UsageTotals {
            input_tokens: u64::try_from(input).unwrap_or(0),
            output_tokens: u64::try_from(output).unwrap_or(0),
            cached_tokens: u64::try_from(cached).unwrap_or(0),
        })
    }

    pub fn insert_artifact(
        &self,
        id: &blora_types::ArtifactId,
        session_id: &SessionId,
        kind: &str,
        path: Option<&str>,
        content: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO artifacts (id, session_id, kind, path, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id.as_str(),
                session_id.as_str(),
                kind,
                path,
                content,
                now.to_rfc3339()
            ],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }

    pub fn list_artifacts(&self, session_id: Option<&SessionId>) -> Result<Vec<ArtifactRecord>> {
        let conn = self.lock();
        let mut out = Vec::new();
        if let Some(session_id) = session_id {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, kind, path, content, created_at FROM artifacts
                     WHERE session_id = ?1 ORDER BY created_at DESC",
                )
                .map_err(BloraError::storage)?;
            let rows = stmt
                .query_map(params![session_id.as_str()], artifact_row)
                .map_err(BloraError::storage)?;
            for row in rows {
                out.push(row_to_artifact(row.map_err(BloraError::storage)?)?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, kind, path, content, created_at FROM artifacts
                     ORDER BY created_at DESC",
                )
                .map_err(BloraError::storage)?;
            let rows = stmt
                .query_map([], artifact_row)
                .map_err(BloraError::storage)?;
            for row in rows {
                out.push(row_to_artifact(row.map_err(BloraError::storage)?)?);
            }
        }
        Ok(out)
    }

    fn list_agents_filtered(
        &self,
        agent_id: Option<&AgentId>,
        parent: Option<&SessionId>,
    ) -> Result<Vec<AgentRecord>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, parent_session_id, child_session_id, role, depth, status,
                        budget_turns, summary, created_at, updated_at
                 FROM agents
                 WHERE (?1 IS NULL OR id = ?1) AND (?2 IS NULL OR parent_session_id = ?2)
                 ORDER BY created_at ASC",
            )
            .map_err(BloraError::storage)?;
        let rows = stmt
            .query_map(
                params![agent_id.map(AgentId::as_str), parent.map(SessionId::as_str)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                },
            )
            .map_err(BloraError::storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, parent_id, child, role, depth, status, budget, summary, created, updated) =
                row.map_err(BloraError::storage)?;
            out.push(AgentRecord {
                id: AgentId::parse(&id)?,
                parent_session_id: SessionId::parse(&parent_id)?,
                child_session_id: SessionId::parse(&child)?,
                role,
                depth: u32::try_from(depth).unwrap_or(0),
                status,
                budget_turns: u32::try_from(budget).unwrap_or(0),
                summary,
                created_at: parse_time(&created)?,
                updated_at: parse_time(&updated)?,
            });
        }
        Ok(out)
    }

    pub fn list_agents(&self, parent: &SessionId) -> Result<Vec<AgentRecord>> {
        self.list_agents_filtered(None, Some(parent))
    }
}

type TaskRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
    i64,
    i64,
    Option<String>,
    String,
    String,
    Option<String>,
);

fn task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
    ))
}

fn row_to_task(row: TaskRow) -> Result<TaskRecord> {
    let (
        id,
        session_id,
        title,
        prompt,
        status,
        run_id,
        delay_until,
        attempt,
        max_attempts,
        auto_approve,
        mock,
        error,
        created_at,
        updated_at,
        cron,
    ) = row;
    Ok(TaskRecord {
        id: TaskId::parse(&id)?,
        session_id: SessionId::parse(&session_id)?,
        title,
        prompt,
        status: TaskStatus::parse(&status)?,
        run_id: run_id.as_deref().map(RunId::parse).transpose()?,
        delay_until: delay_until.as_deref().map(parse_time).transpose()?,
        attempt: u32::try_from(attempt).unwrap_or(0),
        max_attempts: u32::try_from(max_attempts).unwrap_or(3),
        auto_approve: auto_approve != 0,
        mock: mock != 0,
        cron: cron.filter(|value| !value.is_empty()),
        error,
        created_at: parse_time(&created_at)?,
        updated_at: parse_time(&updated_at)?,
    })
}

struct EventRow {
    event_id: String,
    schema_version: i64,
    session_id: String,
    run_id: Option<String>,
    turn_id: Option<String>,
    parent_event_id: Option<String>,
    causation_id: Option<String>,
    sequence: i64,
    timestamp: String,
    actor: String,
    visibility: String,
    event_type: String,
    payload: String,
    metadata: String,
}

impl EventRow {
    fn into_envelope(self) -> Result<EventEnvelope> {
        Ok(EventEnvelope {
            event_id: EventId::parse(&self.event_id)?,
            schema_version: u32::try_from(self.schema_version).unwrap_or(SCHEMA_VERSION),
            session_id: SessionId::parse(&self.session_id)?,
            run_id: self.run_id.as_deref().map(RunId::parse).transpose()?,
            turn_id: self.turn_id.as_deref().map(TurnId::parse).transpose()?,
            parent_event_id: self
                .parent_event_id
                .as_deref()
                .map(EventId::parse)
                .transpose()?,
            causation_id: self
                .causation_id
                .as_deref()
                .map(EventId::parse)
                .transpose()?,
            sequence: u64::try_from(self.sequence).unwrap_or(0),
            timestamp: parse_time(&self.timestamp)?,
            actor: Actor::parse(&self.actor)?,
            visibility: Visibility::parse(&self.visibility)?,
            event_type: self.event_type,
            payload: serde_json::from_str(&self.payload).map_err(BloraError::storage)?,
            metadata: serde_json::from_str(&self.metadata).unwrap_or_else(|_| BTreeMap::new()),
        })
    }
}

fn insert_event(tx: &rusqlite::Transaction<'_>, event: &EventEnvelope) -> Result<()> {
    tx.execute(
        "INSERT INTO events (
            session_id, sequence, event_id, schema_version, run_id, turn_id,
            parent_event_id, causation_id, timestamp, actor, visibility,
            event_type, payload, metadata
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            event.session_id.as_str(),
            event.sequence as i64,
            event.event_id.as_str(),
            event.schema_version as i64,
            event.run_id.as_ref().map(RunId::as_str),
            event.turn_id.as_ref().map(TurnId::as_str),
            event.parent_event_id.as_ref().map(EventId::as_str),
            event.causation_id.as_ref().map(EventId::as_str),
            event.timestamp.to_rfc3339(),
            event.actor.as_str(),
            event.visibility.as_str(),
            event.event_type,
            serde_json::to_string(&event.payload).map_err(BloraError::storage)?,
            serde_json::to_string(&event.metadata).map_err(BloraError::storage)?,
        ],
    )
    .map_err(BloraError::storage)?;
    Ok(())
}

fn next_sequence(tx: &rusqlite::Transaction<'_>, session_id: &SessionId) -> Result<u64> {
    let max: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE session_id = ?1",
            params![session_id.as_str()],
            |row| row.get(0),
        )
        .map_err(BloraError::storage)?;
    Ok(u64::try_from(max).unwrap_or(0) + 1)
}

fn session_exists(tx: &rusqlite::Transaction<'_>, session_id: &SessionId) -> Result<()> {
    let found: Option<String> = tx
        .query_row(
            "SELECT id FROM sessions WHERE id = ?1",
            params![session_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(BloraError::storage)?;
    if found.is_none() {
        return Err(BloraError::SessionNotFound(session_id.to_string()));
    }
    Ok(())
}

fn apply_denormalized(tx: &rusqlite::Transaction<'_>, event: &EventEnvelope) -> Result<()> {
    tx.execute(
        "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
        params![event.timestamp.to_rfc3339(), event.session_id.as_str()],
    )
    .map_err(BloraError::storage)?;

    if let Some(payload) = event.decode_payload()? {
        match payload {
            KnownPayload::SessionArchived(_) => {
                tx.execute(
                    "UPDATE sessions SET status = 'archived', updated_at = ?1 WHERE id = ?2",
                    params![event.timestamp.to_rfc3339(), event.session_id.as_str()],
                )
                .map_err(BloraError::storage)?;
            }
            KnownPayload::UsageRecorded(usage) => {
                tx.execute(
                    "INSERT INTO provider_usage (session_id, run_id, input_tokens, output_tokens, cached_tokens, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        event.session_id.as_str(),
                        event.run_id.as_ref().map(RunId::as_str),
                        usage.input_tokens as i64,
                        usage.output_tokens as i64,
                        usage.cached_tokens as i64,
                        event.timestamp.to_rfc3339(),
                    ],
                )
                .map_err(BloraError::storage)?;
            }
            _ => {}
        }
    }

    if let Some(KnownPayload::RunCreated(created)) = event.decode_payload()? {
        let run_id = event
            .run_id
            .as_ref()
            .ok_or_else(|| BloraError::event("run.created requires run_id"))?;
        tx.execute(
            "INSERT INTO runs (id, session_id, status, mode, model, cancel_requested, created_at, updated_at)
             VALUES (?1, ?2, 'queued', ?3, ?4, 0, ?5, ?6)",
            params![
                run_id.as_str(),
                event.session_id.as_str(),
                created.mode.as_str(),
                created.model,
                event.timestamp.to_rfc3339(),
                event.timestamp.to_rfc3339(),
            ],
        )
        .map_err(BloraError::storage)?;
        return Ok(());
    }

    if let Some(run_id) = &event.run_id {
        if let Some(payload) = event.decode_payload()? {
            if matches!(payload, KnownPayload::RunCancelRequested(_)) {
                tx.execute(
                    "UPDATE runs SET cancel_requested = 1, updated_at = ?1 WHERE id = ?2",
                    params![event.timestamp.to_rfc3339(), run_id.as_str()],
                )
                .map_err(BloraError::storage)?;
            }
            if let Some(status) = status_from_payload(&payload) {
                tx.execute(
                    "UPDATE runs SET status = ?1, updated_at = ?2 WHERE id = ?3",
                    params![status, event.timestamp.to_rfc3339(), run_id.as_str()],
                )
                .map_err(BloraError::storage)?;
            }
        }
    }
    Ok(())
}

fn status_from_payload(payload: &KnownPayload) -> Option<&'static str> {
    match payload {
        KnownPayload::RunStarted(_) | KnownPayload::ApprovalResolved(_) => Some("running"),
        KnownPayload::RunPaused(_) => Some("paused"),
        KnownPayload::ApprovalRequested(_) => Some("waiting_approval"),
        KnownPayload::ContextCompactionStarted(_) => Some("compacting"),
        KnownPayload::ContextCompactionCompleted(_) => Some("running"),
        KnownPayload::RunCompleted(_) => Some("completed"),
        KnownPayload::RunFailed(_) => Some("failed"),
        KnownPayload::RunCancelled(_) => Some("cancelled"),
        _ => None,
    }
}

fn restrict_home(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o700);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    let _ = path;
}

type ArtifactRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
);

fn artifact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn row_to_artifact(row: ArtifactRow) -> Result<ArtifactRecord> {
    let (id, session_id, kind, path, content, created_at) = row;
    Ok(ArtifactRecord {
        id: ArtifactId::parse(&id)?,
        session_id: SessionId::parse(&session_id)?,
        kind,
        path,
        content,
        created_at: parse_time(&created_at)?,
    })
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(BloraError::storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_events::UserInput;
    use blora_session::TranscriptItem;

    #[test]
    fn persist_reopen_and_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = SqliteStore::open(&path).unwrap();
            let mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        let session_id = {
            let store = SqliteStore::open(&path).unwrap();
            let session_id = store
                .create_session(CreateSession {
                    title: Some("one".to_owned()),
                    workspace_path: "/tmp/ws".to_owned(),
                    mode: Mode::Code,
                    parent_session_id: None,
                })
                .unwrap();
            store
                .append(NewEvent::new(
                    session_id.clone(),
                    KnownPayload::UserInput(UserInput {
                        text: "hello".to_owned(),
                    }),
                ))
                .unwrap();
            session_id
        };

        let reopened = SqliteStore::open(&path).unwrap();
        let projection = reopened.load_projection(&session_id).unwrap();
        assert_eq!(projection.last_sequence, 2);
        assert!(matches!(
            projection.transcript[0],
            TranscriptItem::User { .. }
        ));
    }

    #[test]
    fn events_are_append_only() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .create_session(CreateSession {
                title: None,
                workspace_path: "/tmp/ws".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        let conn = store.lock();
        let err = conn
            .execute("UPDATE events SET event_type = 'x' WHERE sequence = 1", [])
            .unwrap_err();
        assert!(err.to_string().contains("append-only"));
    }
}
