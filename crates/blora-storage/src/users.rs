// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_types::{BloraError, Result, SessionId};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::SqliteStore;

#[derive(Clone, Debug)]
pub struct UserRecord {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
}

#[must_use]
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl SqliteStore {
    pub fn create_user(&self, name: &str) -> Result<(UserRecord, String)> {
        let name = name.trim();
        if name.is_empty() {
            return Err(BloraError::Other("user name is empty".to_owned()));
        }
        let id = format!("usr_{}", uuid::Uuid::now_v7().simple());
        let token = format!("blt_{}", uuid::Uuid::now_v7().simple());
        let token_hash = hash_token(&token);
        let now = Utc::now();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO users (id, name, token_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, token_hash, now.to_rfc3339()],
        )
        .map_err(BloraError::storage)?;
        Ok((
            UserRecord {
                id,
                name: name.to_owned(),
                token_hash,
                created_at: now,
            },
            token,
        ))
    }

    pub fn user_by_token(&self, token: &str) -> Result<Option<UserRecord>> {
        let hash = hash_token(token);
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT id, name, token_hash, created_at FROM users WHERE token_hash = ?1",
                params![hash],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(BloraError::storage)?;
        row.map(|(id, name, token_hash, created)| {
            Ok(UserRecord {
                id,
                name,
                token_hash,
                created_at: DateTime::parse_from_rfc3339(&created)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(BloraError::storage)?,
            })
        })
        .transpose()
    }

    pub fn list_users(&self) -> Result<Vec<UserRecord>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT id, name, token_hash, created_at FROM users ORDER BY created_at ASC")
            .map_err(BloraError::storage)?;
        let rows = stmt
            .query_map([], |row| {
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
            let (id, name, token_hash, created) = row.map_err(BloraError::storage)?;
            out.push(UserRecord {
                id,
                name,
                token_hash,
                created_at: DateTime::parse_from_rfc3339(&created)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(BloraError::storage)?,
            });
        }
        Ok(out)
    }

    pub fn session_user_id(&self, session_id: &SessionId) -> Result<Option<String>> {
        let conn = self.lock();
        conn.query_row(
            "SELECT user_id FROM sessions WHERE id = ?1",
            params![session_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(BloraError::storage)
    }

    pub fn set_session_user(&self, session_id: &SessionId, user_id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE sessions SET user_id = ?1 WHERE id = ?2",
            params![user_id, session_id.as_str()],
        )
        .map_err(BloraError::storage)?;
        Ok(())
    }

    pub fn list_sessions_for_user(
        &self,
        user_id: Option<&str>,
    ) -> Result<Vec<crate::SessionSummary>> {
        match user_id {
            Some(user_id) => {
                let all = self.list_sessions()?;
                let conn = self.lock();
                let mut stmt = conn
                    .prepare("SELECT id FROM sessions WHERE user_id = ?1")
                    .map_err(BloraError::storage)?;
                let ids: Vec<String> = stmt
                    .query_map(params![user_id], |row| row.get(0))
                    .map_err(BloraError::storage)?
                    .filter_map(std::result::Result::ok)
                    .collect();
                Ok(all
                    .into_iter()
                    .filter(|session| ids.iter().any(|id| id == session.id.as_str()))
                    .collect())
            }
            None => self.list_sessions(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::SqliteStore;

    #[test]
    fn creates_and_authenticates_user() {
        let store = SqliteStore::open_in_memory().unwrap();
        let (user, token) = store.create_user("alice").unwrap();
        assert!(token.starts_with("blt_"));
        let found = store.user_by_token(&token).unwrap().unwrap();
        assert_eq!(found.id, user.id);
        assert!(store.user_by_token("blt_nope").unwrap().is_none());
    }
}
