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
}

#[must_use]
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl SqliteStore {
    pub fn upsert_passport_user(
        &self,
        username: &str,
        nickname: Option<&str>,
        avatar: Option<&str>,
        email: Option<&str>,
        app_token: Option<&str>,
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
            "INSERT INTO users (id, name, token_hash, created_at, passport_username, passport_nickname, passport_avatar, passport_email, passport_app_token)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(passport_username) DO UPDATE SET name=excluded.name, passport_nickname=excluded.passport_nickname, passport_avatar=excluded.passport_avatar, passport_email=excluded.passport_email, passport_app_token=excluded.passport_app_token",
            params![id, name, token_hash, created_at, username, nickname, avatar, email, app_token],
        )
        .map_err(BloraError::storage)?;
        self.user_by_passport_username(username)?
            .ok_or_else(|| BloraError::Other("Passport user was not stored".to_owned()))
    }

    pub fn clear_passport_users(&self) -> Result<usize> {
        let conn = self.lock();
        conn.execute("DELETE FROM users WHERE passport_username IS NOT NULL", [])
            .map_err(BloraError::storage)
    }

    pub fn user_by_passport_username(&self, username: &str) -> Result<Option<UserRecord>> {
        let conn = self.lock();
        let row = conn.query_row(
            "SELECT id, name, token_hash, created_at, passport_username, passport_nickname, passport_avatar, passport_email, passport_app_token FROM users WHERE passport_username = ?1",
            params![username],
            |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, Option<String>>(6)?, row.get::<_, Option<String>>(7)?, row.get::<_, Option<String>>(8)?,
            )),
        ).optional().map_err(BloraError::storage)?;
        row.map(
            |(
                id,
                name,
                token_hash,
                created,
                passport_username,
                passport_nickname,
                passport_avatar,
                passport_email,
                passport_app_token,
            )| {
                Ok(UserRecord {
                    id,
                    name,
                    token_hash,
                    created_at: DateTime::parse_from_rfc3339(&created)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(BloraError::storage)?,
                    passport_username,
                    passport_nickname,
                    passport_avatar,
                    passport_email,
                    passport_app_token,
                })
            },
        )
        .transpose()
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
            },
            token,
        ))
    }

    pub fn user_by_token(&self, token: &str) -> Result<Option<UserRecord>> {
        let hash = hash_token(token);
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT id, name, token_hash, created_at, passport_username, passport_nickname, passport_avatar, passport_email, passport_app_token FROM users WHERE token_hash = ?1",
                params![hash],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                },
            )
            .optional()
            .map_err(BloraError::storage)?;
        row.map(
            |(
                id,
                name,
                token_hash,
                created,
                passport_username,
                passport_nickname,
                passport_avatar,
                passport_email,
                passport_app_token,
            )| {
                Ok(UserRecord {
                    id,
                    name,
                    token_hash,
                    created_at: DateTime::parse_from_rfc3339(&created)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(BloraError::storage)?,
                    passport_username,
                    passport_nickname,
                    passport_avatar,
                    passport_email,
                    passport_app_token,
                })
            },
        )
        .transpose()
    }

    pub fn list_users(&self) -> Result<Vec<UserRecord>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT id, name, token_hash, created_at, passport_username, passport_nickname, passport_avatar, passport_email, passport_app_token FROM users ORDER BY created_at ASC")
            .map_err(BloraError::storage)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })
            .map_err(BloraError::storage)?;
        let mut out = Vec::new();
        for row in rows {
            let (
                id,
                name,
                token_hash,
                created,
                passport_username,
                passport_nickname,
                passport_avatar,
                passport_email,
                passport_app_token,
            ) = row.map_err(BloraError::storage)?;
            out.push(UserRecord {
                id,
                name,
                token_hash,
                created_at: DateTime::parse_from_rfc3339(&created)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(BloraError::storage)?,
                passport_username,
                passport_nickname,
                passport_avatar,
                passport_email,
                passport_app_token,
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
