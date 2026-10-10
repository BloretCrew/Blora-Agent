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
    // File-backed UI searches use an independent reader so scans cannot hold
    // the foreground event store mutex. In-memory tests share their connection.
    fn search_reader(&self) -> Result<Option<Connection>> {
        let path = self
            .lock()
            .path()
            .filter(|path| !path.is_empty())
            .map(str::to_owned);
        path.map(|path| {
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(BloraError::storage)
        })
        .transpose()
    }

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
        self.search_transcripts_page_inner(query, limit, exclude, user_id, None, true)
            .map(|(matches, _)| matches)
    }

    /// Search all sessions, including child sessions, in batches of at most 50.
    /// Resume after the returned `(at, rowid)` cursor using the same filters.
    pub fn search_transcripts_page(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
        cursor: Option<&(String, i64)>,
    ) -> Result<(Vec<SearchMatch>, Option<(String, i64)>)> {
        self.search_transcripts_page_inner(query, limit, exclude, user_id, cursor, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_transcripts_page_inner(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
        cursor: Option<&(String, i64)>,
        roots_only: bool,
    ) -> Result<(Vec<SearchMatch>, Option<(String, i64)>)> {
        let (fts, short) = fts_query(query);
        if fts.is_none() && short.is_empty() {
            return Ok((Vec::new(), None));
        }
        let reader = self.search_reader()?;
        let shared = reader.is_none().then(|| self.lock());
        let conn = reader.as_ref().unwrap_or_else(|| shared.as_ref().unwrap());
        let mut sql = String::from(
            "SELECT f.session_id, s.title, f.kind, f.at, f.body, f.rowid
             FROM transcript_fts f JOIN sessions s ON s.id = f.session_id
             WHERE 1 = 1",
        );
        if roots_only {
            sql.push_str(" AND s.parent_session_id IS NULL");
        }
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
        if let Some((at, rowid)) = cursor {
            args.push(at.clone());
            let at_param = args.len();
            args.push(rowid.to_string());
            sql.push_str(&format!(
                " AND (f.at < ?{at_param} OR (f.at = ?{at_param} AND f.rowid < ?{}))",
                args.len()
            ));
        }
        let limit = limit.clamp(1, 50);
        args.push((limit + 1).to_string());
        sql.push_str(&format!(
            " ORDER BY f.at DESC, f.rowid DESC LIMIT ?{}",
            args.len()
        ));
        let mut stmt = conn.prepare(&sql).map_err(BloraError::storage)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(BloraError::storage)?;
        let needles: Vec<String> = query
            .split_whitespace()
            .map(|term| term.replace('"', "").to_lowercase())
            .filter(|term| !term.is_empty())
            .collect();
        let mut out = Vec::new();
        let mut next_cursor = None;
        for row in rows {
            let (session_id, title, kind, at, body, rowid) = row.map_err(BloraError::storage)?;
            if out.len() == limit {
                return Ok((out, next_cursor));
            }
            next_cursor = Some((at.clone(), rowid));
            out.push(SearchMatch {
                session_id,
                title,
                kind,
                at,
                snippet: snippet(&body, &needles),
            });
        }
        Ok((out, None))
    }

    /// Latest nonempty user/assistant preview and latest event sequence per ID.
    /// Existing sessions without a message have a `None` preview; missing IDs
    /// are omitted. Results follow the input order, including duplicate IDs.
    pub fn session_message_previews(
        &self,
        session_ids: &[SessionId],
    ) -> Result<Vec<(SessionId, u64, Option<String>)>> {
        let reader = self.search_reader()?;
        let shared = reader.is_none().then(|| self.lock());
        let conn = reader.as_ref().unwrap_or_else(|| shared.as_ref().unwrap());
        let mut out = Vec::new();
        let whitespace: String = (0..=0x3000)
            .filter_map(char::from_u32)
            .filter(|ch| ch.is_whitespace())
            .collect();
        // Keep each batch below SQLite's default variable limit.
        for ids in session_ids.chunks(400) {
            let placeholders = (1..=ids.len())
                .map(|index| format!("(?{index}, {index})"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "WITH requested(id, position) AS (VALUES {placeholders})
                 SELECT s.id,
                        COALESCE((SELECT MAX(sequence) FROM events WHERE session_id = s.id), 0),
                        (SELECT json_extract(e.payload, '$.text') FROM events e
                         WHERE e.session_id = s.id
                           AND e.event_type IN ('user.input', 'assistant.message.completed')
                           AND json_type(e.payload, '$.text') = 'text'
                           AND length(trim(json_extract(e.payload, '$.text'), ?{})) > 0
                         ORDER BY e.sequence DESC LIMIT 1)
                 FROM requested r JOIN sessions s ON s.id = r.id
                 ORDER BY r.position",
                ids.len() + 1
            );
            let mut args: Vec<&str> = ids.iter().map(SessionId::as_str).collect();
            args.push(&whitespace);
            let mut stmt = conn.prepare(&sql).map_err(BloraError::storage)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(args), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(BloraError::storage)?;
            for row in rows {
                let (id, sequence, text) = row.map_err(BloraError::storage)?;
                out.push((
                    SessionId::parse(id)?,
                    sequence,
                    text.map(|text| snippet(&text, &[])),
                ));
            }
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
    fn file_search_reader_does_not_hold_foreground_connection() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(directory.path().join("sessions.sqlite")).unwrap();
        let id = session(&store, "title", &["中文部署 needle"]);
        let reader = store.search_reader().unwrap().unwrap();
        let _foreground = store.lock();
        let count: i64 = reader
            .query_row("SELECT COUNT(*) FROM transcript_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        drop(_foreground);
        let (hits, _) = store
            .search_transcripts_page("部署", 20, None, None, None)
            .unwrap();
        assert_eq!(hits[0].session_id, id.as_str());
        assert!(
            store.session_message_previews(&[id]).unwrap()[0]
                .2
                .as_ref()
                .unwrap()
                .contains("部署")
        );
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
    fn paginates_short_chinese_terms_with_identical_timestamps() {
        let store = SqliteStore::open_in_memory().unwrap();
        let texts: Vec<String> = (0..123).map(|index| format!("部署记录 {index}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let id = session(&store, "部署", &refs);
        store
            .lock()
            .execute(
                "UPDATE transcript_fts SET at = '2026-10-10T00:00:00+00:00'",
                [],
            )
            .unwrap();
        let mut cursor = None;
        let mut snippets = Vec::new();
        let mut sizes = Vec::new();
        loop {
            let (hits, next) = store
                .search_transcripts_page("部署", 500, None, None, cursor.as_ref())
                .unwrap();
            sizes.push(hits.len());
            assert!(hits.iter().all(|hit| hit.session_id == id.as_str()));
            snippets.extend(hits.into_iter().map(|hit| hit.snippet));
            if let Some(previous) = &cursor {
                if let Some(next) = &next {
                    assert!(next < previous);
                }
            }
            cursor = next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(sizes, vec![50, 50, 23]);
        assert_eq!(snippets, texts.into_iter().rev().collect::<Vec<_>>());
        assert_eq!(
            store
                .search_transcripts("部署", 500, None, None)
                .unwrap()
                .len(),
            50
        );
        assert!(
            store
                .search_transcripts_page(" ", 50, None, None, None)
                .unwrap()
                .1
                .is_none()
        );
    }

    #[test]
    fn page_filters_and_child_sessions_preserve_legacy_search() {
        let store = SqliteStore::open_in_memory().unwrap();
        let root = session(&store, "root", &["部署 nginx", "部署 nginx again"]);
        let child = store
            .create_session(crate::CreateSession {
                title: None,
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Agent,
                parent_session_id: Some(root.clone()),
            })
            .unwrap();
        store
            .append(NewEvent::new(
                child.clone(),
                KnownPayload::UserInput(UserInput {
                    text: "部署 nginx child".to_owned(),
                }),
            ))
            .unwrap();
        let (user, _) = store.create_user("search-user").unwrap();
        store.set_session_user(&root, &user.id).unwrap();
        assert_eq!(
            store
                .search_transcripts("部署 nginx", 50, None, None)
                .unwrap()
                .len(),
            2
        );
        let (hits, cursor) = store
            .search_transcripts_page("部署 nginx", 50, None, None, None)
            .unwrap();
        assert_eq!(hits.len(), 3);
        assert!(cursor.is_none());
        let (hits, _) = store
            .search_transcripts_page("部署 nginx", 50, Some(&root), None, None)
            .unwrap();
        assert_eq!(hits[0].session_id, child.as_str());
        let (hits, cursor) = store
            .search_transcripts_page("部署 nginx", 1, None, Some(&user.id), None)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, root.as_str());
        let (next, cursor) = store
            .search_transcripts_page("部署 nginx", 1, None, Some(&user.id), cursor.as_ref())
            .unwrap();
        assert_eq!(next.len(), 1);
        assert!(cursor.is_none());
    }

    #[test]
    fn previews_use_latest_nonempty_canonical_message_and_last_event() {
        use blora_events::{AssistantMessageCompleted, AssistantReasoning};
        let store = SqliteStore::open_in_memory().unwrap();
        let id = session(&store, "preview", &["first"]);
        store
            .append(NewEvent::new(
                id.clone(),
                KnownPayload::AssistantMessageCompleted(AssistantMessageCompleted {
                    text: "最新助手回复".to_owned(),
                }),
            ))
            .unwrap();
        store
            .append(NewEvent::new(
                id.clone(),
                KnownPayload::UserInput(UserInput {
                    text: " \t\n\u{3000}".to_owned(),
                }),
            ))
            .unwrap();
        store
            .append(NewEvent::new(
                id.clone(),
                KnownPayload::AssistantReasoning(AssistantReasoning {
                    text: "not a preview".to_owned(),
                }),
            ))
            .unwrap();
        let empty = session(&store, "empty", &[]);
        store
            .lock()
            .execute("DELETE FROM transcript_fts", [])
            .unwrap();
        let requested = vec![id.clone(), empty.clone(), SessionId::generate(), id.clone()];
        assert_eq!(
            store.session_message_previews(&requested).unwrap(),
            vec![
                (id.clone(), 5, Some("最新助手回复".to_owned())),
                (empty, 1, None),
                (id.clone(), 5, Some("最新助手回复".to_owned())),
            ]
        );
        let many = vec![id; 401];
        assert_eq!(store.session_message_previews(&many).unwrap().len(), 401);
        assert!(store.session_message_previews(&[]).unwrap().is_empty());
    }

    #[test]
    fn snippet_centres_on_the_match() {
        let body = format!("{} needle {}", "a".repeat(300), "b".repeat(300));
        let text = snippet(&body, &["needle".to_owned()]);
        assert!(text.starts_with('…') && text.ends_with('…') && text.contains("needle"));
    }
}
