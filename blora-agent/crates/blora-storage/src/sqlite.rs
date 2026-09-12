// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use blora_events::{EventEnvelope, KnownPayload, NewEvent, SessionCreated};
use blora_session::{SessionProjection, rebuild};
use blora_types::{
    Actor, BloraError, Clock, EventId, Mode, Result, RunId, SCHEMA_VERSION, SessionId,
    SessionStatus, SystemClock, TurnId, Visibility,
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
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (1, ?1)",
            params![Utc::now().to_rfc3339()],
        )
        .map_err(BloraError::storage)?;
        Ok(Self {
            conn: Mutex::new(conn),
            clock: SystemClock,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
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
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT event_id, schema_version, session_id, run_id, turn_id, parent_event_id,
                        causation_id, sequence, timestamp, actor, visibility, event_type, payload, metadata
                 FROM events WHERE session_id = ?1 ORDER BY sequence ASC",
            )
            .map_err(BloraError::storage)?;
        let rows = stmt
            .query_map(params![session_id.as_str()], |row| {
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
