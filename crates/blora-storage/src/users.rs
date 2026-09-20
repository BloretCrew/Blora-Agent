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
    pub passport_username: Option<String>,
    pub passport_nickname: Option<String>,
    pub passport_avatar: Option<String>,
    pub passport_email: Option<String>,
    pub passport_app_token: Option<String>,
    pub passport_refresh_token: Option<String>,
    pub passport_token_expires_at: Option<DateTime<Utc>>,
}

#[must_use]
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

const USER_COLUMNS: &str = "id, name, token_hash, created_at, passport_username, passport_nickname, passport_avatar, passport_email, passport_app_token, passport_refresh_token, passport_token_expires_at";

fn map_user_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserRecord> {
    let created: String = row.get(3)?;
    let created_at = DateTime::parse_from_rfc3339(&created)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| {
            rusqlite::Error::InvalidColumnType(3, err.to_string(), rusqlite::types::Type::Text)
        })?;
    let expires: Option<String> = row.get(10)?;
    let passport_token_expires_at = expires
        .as_deref()
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|err| {
                    rusqlite::Error::InvalidColumnType(
                        10,
                        err.to_string(),
                        rusqlite::types::Type::Text,
                    )
                })
        })
        .transpose()?;
    Ok(UserRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        token_hash: row.get(2)?,
        created_at,
        passport_username: row.get(4)?,
        passport_nickname: row.get(5)?,
        passport_avatar: row.get(6)?,
        passport_email: row.get(7)?,
        passport_app_token: row.get(8)?,
        passport_refresh_token: row.get(9)?,
        passport_token_expires_at,
    })
}

impl SqliteStore {
    pub fn upsert_passport_user(
        &self,
        username: &str,
        nickname: Option<&str>,
        avatar: Option<&str>,
        email: Option<&str>,
        app_token: Option<&str>,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<UserRecord> {
        let username = username.trim();
        if username.is_empty() {
            return Err(BloraError::Other("Passport username is empty".to_owned()));
        }
        let now = Utc::now();
        let conn = self.lock();
        let existing = conn
            .query_row(
                "SELECT id, name, token_hash, created_at FROM users WHERE passport_username = ?1",
                params![username],
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
        let id = existing
            .as_ref()
            .map(|row| row.0.clone())
            .unwrap_or_else(|| format!("usr_{}", uuid::Uuid::now_v7().simple()));
        let name = nickname
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(username);
        let token_hash = existing
            .as_ref()
            .map(|row| row.2.clone())
            .unwrap_or_else(|| {
                hash_token(&format!("passport:{username}:{}", uuid::Uuid::now_v7()))
            });
        let created_at = existing
            .as_ref()
            .map(|row| row.3.clone())
            .unwrap_or_else(|| now.to_rfc3339());
        conn.execute(
            "INSERT INTO users (id, name, token_hash, created_at, passport_username, passport_nickname, passport_avatar, passport_email, passport_app_token, passport_refresh_token, passport_token_expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(passport_username) DO UPDATE SET name=excluded.name, passport_nickname=excluded.passport_nickname, passport_avatar=excluded.passport_avatar, passport_email=excluded.passport_email, passport_app_token=excluded.passport_app_token, passport_refresh_token=COALESCE(excluded.passport_refresh_token, passport_refresh_token), passport_token_expires_at=COALESCE(excluded.passport_token_expires_at, passport_token_expires_at)",
            params![
                id,
                name,
                token_hash,
                created_at,
                username,
                nickname,
                avatar,
                email,
                app_token,
                refresh_token,
                token_expires_at.map(|value| value.to_rfc3339())
            ],
        )
        .map_err(BloraError::storage)?;
        // Must not call user_by_passport_username here: it re-locks the same
        // non-reentrant connection mutex and would deadlock the caller.
        let row = conn
            .query_row(
                &format!("SELECT {USER_COLUMNS} FROM users WHERE passport_username = ?1"),
                params![username],
                map_user_row,
            )
            .optional()
            .map_err(BloraError::storage)?;
        row.ok_or_else(|| BloraError::Other("Passport user was not stored".to_owned()))
    }

    pub fn update_passport_tokens(
        &self,
        username: &str,
        app_token: &str,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<Option<UserRecord>> {
        let conn = self.lock();
        let changed = conn
            .execute(
                "UPDATE users SET passport_app_token = ?1, passport_refresh_token = COALESCE(?2, passport_refresh_token), passport_token_expires_at = ?3 WHERE passport_username = ?4",
                params![
                    app_token,
                    refresh_token,
                    token_expires_at.map(|value| value.to_rfc3339()),
                    username.trim()
                ],
            )
            .map_err(BloraError::storage)?;
        if changed == 0 {
            return Ok(None);
        }
        conn.query_row(
            &format!("SELECT {USER_COLUMNS} FROM users WHERE passport_username = ?1"),
            params![username.trim()],
            map_user_row,
        )
        .optional()
        .map_err(BloraError::storage)
    }

    pub fn clear_passport_users(&self) -> Result<usize> {
        let conn = self.lock();
        conn.execute("DELETE FROM users WHERE passport_username IS NOT NULL", [])
            .map_err(BloraError::storage)
    }

    pub fn user_by_passport_username(&self, username: &str) -> Result<Option<UserRecord>> {
        let conn = self.lock();
        let row = conn
            .query_row(
                &format!("SELECT {USER_COLUMNS} FROM users WHERE passport_username = ?1"),
                params![username],
                map_user_row,
            )
            .optional()
            .map_err(BloraError::storage)?;
        Ok(row)
    }

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
                passport_username: None,
                passport_nickname: None,
                passport_avatar: None,
                passport_email: None,
                passport_app_token: None,
                passport_refresh_token: None,
                passport_token_expires_at: None,
            },
            token,
        ))
    }

    pub fn user_by_token(&self, token: &str) -> Result<Option<UserRecord>> {
        let hash = hash_token(token);
        let conn = self.lock();
        let row = conn
            .query_row(
                &format!("SELECT {USER_COLUMNS} FROM users WHERE token_hash = ?1"),
                params![hash],
                map_user_row,
            )
            .optional()
            .map_err(BloraError::storage)?;
        Ok(row)
    }

    pub fn list_users(&self) -> Result<Vec<UserRecord>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(&format!("SELECT {USER_COLUMNS} FROM users ORDER BY created_at ASC"))
            .map_err(BloraError::storage)?;
        let rows = stmt
            .query_map([], map_user_row)
            .map_err(BloraError::storage)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(BloraError::storage)?);
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

    #[test]
    fn upsert_preserves_existing_refresh_token_when_login_omits_one() {
        let store = SqliteStore::open_in_memory().unwrap();
        let first = store
            .upsert_passport_user(
                "rhedar",
                Some("Rhedar"),
                None,
                None,
                Some("access-1"),
                Some("refresh-1"),
                None,
            )
            .unwrap();
        assert_eq!(first.passport_refresh_token.as_deref(), Some("refresh-1"));
        let second = store
            .upsert_passport_user(
                "rhedar",
                Some("Rhedar"),
                None,
                None,
                Some("access-2"),
                None,
                None,
            )
            .unwrap();
        assert_eq!(second.passport_app_token.as_deref(), Some("access-2"));
        assert_eq!(second.passport_refresh_token.as_deref(), Some("refresh-1"));
    }

    #[test]
    fn upsert_passport_user_does_not_deadlock_on_connection_mutex() {
        // Regression: upsert used to call user_by_passport_username while
        // still holding the connection lock, which re-locks the same
        // non-reentrant mutex and hangs the caller forever.
        let store = std::sync::Arc::new(SqliteStore::open_in_memory().unwrap());
        std::thread::scope(|scope| {
            let store_for_reader = store.clone();
            let reader = scope.spawn(move || {
                for _ in 0..50 {
                    // Interleave reader traffic so a self-deadlock surfaces as
                    // this loop never completing rather than a silent hang.
                    let _ = store_for_reader.user_by_passport_username("rhedar");
                }
            });
            let store_for_upsert = store.clone();
            let upsert = scope.spawn(move || {
                for round in 0..10 {
                    let record = store_for_upsert
                        .upsert_passport_user(
                            "rhedar",
                            Some(&format!("Rhedar {round}")),
                            None,
                            None,
                            None,
                            None,
                            None,
                        )
                        .unwrap();
                    assert_eq!(record.passport_username.as_deref(), Some("rhedar"));
                }
            });
            reader.join().unwrap();
            upsert.join().unwrap();
        });
        let stored = store
            .user_by_passport_username("rhedar")
            .unwrap()
            .expect("user stored");
        assert_eq!(stored.passport_nickname.as_deref(), Some("Rhedar 9"));
    }
}
