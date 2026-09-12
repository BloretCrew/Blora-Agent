// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_types::{ApprovalId, BloraError, Result, RunId, SessionId};
use chrono::{DateTime, Utc};
use rusqlite::params;

use crate::SqliteStore;

#[derive(Clone, Debug)]
pub struct ApprovalRecord {
    pub id: ApprovalId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub capability: String,
    pub summary: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SqliteStore {
    pub fn insert_approval(&self, record: &ApprovalRecord) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO approvals (id, session_id, run_id, capability, summary, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record.id.as_str(),
                record.session_id.as_str(),
                record.run_id.as_ref().map(RunId::as_str),
                record.capability,
                record.summary,
                record.status,
                record.created_at.to_rfc3339(),
                record.updated_at.to_rfc3339(),
            ],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }

    pub fn get_approval(&self, id: &ApprovalId) -> Result<ApprovalRecord> {
        let conn = self.lock();
        conn.query_row(
            "SELECT id, session_id, run_id, capability, summary, status, created_at, updated_at
             FROM approvals WHERE id = ?1",
            params![id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .map_err(|err| BloraError::storage(err.to_string()))
        .and_then(row_to_approval)
    }

    pub fn list_pending_approvals(&self, session_id: &SessionId) -> Result<Vec<ApprovalRecord>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, run_id, capability, summary, status, created_at, updated_at
                 FROM approvals WHERE session_id = ?1 AND status = 'pending' ORDER BY created_at ASC",
            )
            .map_err(BloraError::storage)?;
        let rows = stmt
            .query_map(params![session_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(BloraError::storage)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row_to_approval(row.map_err(BloraError::storage)?)?);
        }
        Ok(out)
    }

    pub fn resolve_approval_row(
        &self,
        id: &ApprovalId,
        decision: &str,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let conn = self.lock();
        let changed = conn
            .execute(
                "UPDATE approvals SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status = 'pending'",
                params![decision, now.to_rfc3339(), id.as_str()],
            )
            .map_err(BloraError::storage)?;
        Ok(changed > 0)
    }

    pub fn insert_checkpoint(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        sequence: u64,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let id = blora_types::CheckpointId::generate();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO checkpoints (id, session_id, run_id, sequence, created_at, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id.as_str(),
                session_id.as_str(),
                run_id.map(RunId::as_str),
                sequence as i64,
                now.to_rfc3339(),
                note,
            ],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(BloraError::storage)
}

fn row_to_approval(
    row: (
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        String,
        String,
    ),
) -> Result<ApprovalRecord> {
    let (id, session_id, run_id, capability, summary, status, created, updated) = row;
    Ok(ApprovalRecord {
        id: ApprovalId::parse(&id)?,
        session_id: SessionId::parse(&session_id)?,
        run_id: run_id.as_deref().map(RunId::parse).transpose()?,
        capability,
        summary,
        status,
        created_at: parse_time(&created)?,
        updated_at: parse_time(&updated)?,
    })
}
