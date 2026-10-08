// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Full-text index over conversation text, used by `session_search`.
//!
//! The index is derived data: it is rebuilt from the event log when missing,
//! so the event log stays the only source of truth.

use blora_events::EventEnvelope;
use blora_types::{BloraError, Result, SessionId};
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::sqlite::SqliteStore;

const CREATE: &str = "CREATE VIRTUAL TABLE IF NOT EXISTS transcript_fts USING fts5(
    body,
    session_id UNINDEXED,
    event_id UNINDEXED,
    kind UNINDEXED,
    at UNINDEXED,
    tokenize = 'trigram'
)";

/// One matching message.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SearchMatch {
    pub session_id: String,
    pub title: Option<String>,
    pub kind: String,
    pub at: String,
    pub snippet: String,
}

/// Searchable text carried by an event, with its kind.
pub(crate) fn indexable(event: &EventEnvelope) -> Option<(&'static str, String)> {
    let text = |key: &str| {
        event
            .payload
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let found = match event.event_type.as_str() {
        "user.input" => ("user", text("text")?),
        "assistant.message.completed" => ("assistant", text("text")?),
        "input.requested" => ("question", text("question")?),
        "input.provided" => ("answer", text("answer")?),
        "context.compaction.completed" => ("summary", text("summary")?),
        _ => return None,
    };
    (!found.1.trim().is_empty()).then_some(found)
}

pub(crate) fn ensure_index(conn: &Connection) -> Result<()> {
    let existed: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'transcript_fts'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(BloraError::storage)?
        > 0;
    conn.execute(CREATE, []).map_err(BloraError::storage)?;
    if !existed {
        backfill(conn)?;
    }
    Ok(())
}

fn backfill(conn: &Connection) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id, event_id, event_type, payload, timestamp FROM events
             WHERE event_type IN ('user.input', 'assistant.message.completed',
                                  'input.requested', 'input.provided',
                                  'context.compaction.completed')",
        )
        .map_err(BloraError::storage)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(BloraError::storage)?;
    for row in rows {
        let (session_id, event_id, event_type, payload, at) = row.map_err(BloraError::storage)?;
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap_or_default();
        let key = match event_type.as_str() {
            "input.requested" => "question",
            "input.provided" => "answer",
            "context.compaction.completed" => "summary",
            _ => "text",
        };
        let kind = match event_type.as_str() {
            "user.input" => "user",
            "assistant.message.completed" => "assistant",
            "input.requested" => "question",
            "input.provided" => "answer",
            _ => "summary",
        };
        if let Some(body) = payload.get(key).and_then(serde_json::Value::as_str)
            && !body.trim().is_empty()
        {
            conn.execute(
                "INSERT INTO transcript_fts (body, session_id, event_id, kind, at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![body, session_id, event_id, kind, at],
            )
            .map_err(BloraError::storage)?;
        }
    }
    Ok(())
}

pub(crate) fn index_event(tx: &rusqlite::Transaction<'_>, event: &EventEnvelope) -> Result<()> {
    if let Some((kind, body)) = indexable(event) {
        tx.execute(
            "INSERT INTO transcript_fts (body, session_id, event_id, kind, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                body,
                event.session_id.as_str(),
                event.event_id.as_str(),
                kind,
                event.timestamp.to_rfc3339()
            ],
        )
        .map_err(BloraError::storage)?;
    }
    Ok(())
}

/// Quote each whitespace-separated term so user text cannot inject FTS syntax.
/// Terms shorter than three characters cannot use the trigram index and are
/// matched with LIKE instead.
fn fts_query(query: &str) -> (Option<String>, Vec<String>) {
    let mut fts = Vec::new();
    let mut short = Vec::new();
    for term in query.split_whitespace() {
        let clean = term.replace('"', "");
        if clean.is_empty() {
            continue;
        }
        if clean.chars().count() >= 3 {
            fts.push(format!("\"{clean}\""));
        } else {
            short.push(clean);
        }
    }
    ((!fts.is_empty()).then(|| fts.join(" AND ")), short)
}

impl SqliteStore {
    /// Search indexed conversation text, newest first. `exclude` skips one
    /// session (normally the one asking). `user_id` limits results to that
    /// user's sessions when set.
    pub fn search_transcripts(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
    ) -> Result<Vec<SearchMatch>> {
        let (fts, short) = fts_query(query);
        if fts.is_none() && short.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let mut sql = String::from(
            "SELECT f.session_id, s.title, f.kind, f.at, f.body
             FROM transcript_fts f JOIN sessions s ON s.id = f.session_id
             WHERE s.parent_session_id IS NULL",
        );
        let mut args: Vec<String> = Vec::new();
        if let Some(fts) = &fts {
            args.push(fts.clone());
            sql.push_str(&format!(" AND transcript_fts MATCH ?{}", args.len()));
        }
        for term in &short {
            args.push(format!("%{}%", term.replace(['%', '_'], "")));
            sql.push_str(&format!(" AND f.body LIKE ?{}", args.len()));
        }
        if let Some(exclude) = exclude {
            args.push(exclude.as_str().to_owned());
            sql.push_str(&format!(" AND f.session_id != ?{}", args.len()));
        }
        if let Some(user) = user_id {
            args.push(user.to_owned());
            sql.push_str(&format!(" AND s.user_id = ?{}", args.len()));
        }
        args.push(limit.clamp(1, 50).to_string());
        sql.push_str(&format!(" ORDER BY f.at DESC LIMIT ?{}", args.len()));
        let mut stmt = conn.prepare(&sql).map_err(BloraError::storage)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(BloraError::storage)?;
        let needles: Vec<String> = query
            .split_whitespace()
            .map(|term| term.replace('"', "").to_lowercase())
            .filter(|term| !term.is_empty())
            .collect();
        let mut out = Vec::new();
        for row in rows {
            let (session_id, title, kind, at, body) = row.map_err(BloraError::storage)?;
            out.push(SearchMatch {
                session_id,
                title,
                kind,
                at,
                snippet: snippet(&body, &needles),
            });
        }
        Ok(out)
    }
}

/// About 240 characters of `body` around the first matching term.
fn snippet(body: &str, needles: &[String]) -> String {
    let chars: Vec<char> = body.chars().collect();
    let lower: Vec<char> = body.to_lowercase().chars().collect();
    let hit = needles
        .iter()
        .filter_map(|needle| {
            let needle: Vec<char> = needle.chars().collect();
            (0..lower.len().saturating_sub(needle.len() - 1).max(1))
                .find(|&at| lower.get(at..at + needle.len()) == Some(&needle[..]))
        })
        .min()
        .unwrap_or(0);
    let start = hit.saturating_sub(80).min(chars.len());
    let end = (start + 240).min(chars.len());
    let mut text: String = chars[start..end].iter().collect();
    text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        text.insert(0, '…');
    }
    if end < chars.len() {
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_events::{KnownPayload, NewEvent, UserInput};
    use blora_types::Mode;

    fn session(store: &SqliteStore, title: &str, texts: &[&str]) -> SessionId {
        let id = store
            .create_session(crate::CreateSession {
                title: Some(title.to_owned()),
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Agent,
                parent_session_id: None,
            })
            .unwrap();
        for text in texts {
            store
                .append(NewEvent::new(
                    id.clone(),
                    KnownPayload::UserInput(UserInput {
                        text: (*text).to_owned(),
                    }),
                ))
                .unwrap();
        }
        id
    }

    #[test]
    fn finds_messages_across_sessions() {
        let store = SqliteStore::open_in_memory().unwrap();
        let deploy = session(&store, "deploy", &["Configure the nginx reverse proxy for staging"]);
        let other = session(&store, "notes", &["部署到预发布环境的步骤", "unrelated"]);
        let hits = store.search_transcripts("nginx proxy", 10, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, deploy.as_str());
        assert_eq!(hits[0].title.as_deref(), Some("deploy"));
        assert!(hits[0].snippet.contains("nginx"));
        let cjk = store.search_transcripts("预发布", 10, None, None).unwrap();
        assert_eq!(cjk[0].session_id, other.as_str());
        assert!(store.search_transcripts("nginx", 10, Some(&deploy), None).unwrap().is_empty());
        assert!(store.search_transcripts("\"", 10, None, None).unwrap().is_empty());
        let short = store.search_transcripts("ng", 10, None, None).unwrap();
        assert!(!short.is_empty(), "short terms fall back to LIKE");
    }

    #[test]
    fn snippet_centres_on_the_match() {
        let body = format!("{} needle {}", "a".repeat(300), "b".repeat(300));
        let text = snippet(&body, &["needle".to_owned()]);
        assert!(text.starts_with('…') && text.ends_with('…') && text.contains("needle"));
    }
}
