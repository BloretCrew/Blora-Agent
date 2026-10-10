// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use blora_storage::SessionSummary;
use blora_types::SessionId;

#[derive(Clone, Debug)]
pub struct PickerRow {
    pub id: SessionId,
    pub content: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionPicker {
    pub fullscreen: bool,
    pub minimized: bool,
    pub selected: usize,
    pub query: String,
    pub searching: bool,
    pub error: Option<String>,
    pub rows: Vec<PickerRow>,
    pub previews: HashMap<String, (u64, String)>,
    pub scroll: Cell<usize>,
    pub generation: u64,
    pub pending_at: Option<Instant>,
    content: Vec<PickerRow>,
    preview_requested: HashMap<String, u64>,
    search_requested: bool,
    preview_retry_at: Option<Instant>,
}

impl SessionPicker {
    pub fn new(sessions: &[SessionSummary], current: Option<&SessionId>, generation: u64) -> Self {
        let mut picker = Self {
            fullscreen: false,
            minimized: false,
            selected: 0,
            query: String::new(),
            searching: false,
            error: None,
            rows: Vec::new(),
            previews: HashMap::new(),
            scroll: Cell::new(0),
            generation,
            pending_at: None,
            content: Vec::new(),
            preview_requested: HashMap::new(),
            search_requested: false,
            preview_retry_at: None,
        };
        picker.refresh(sessions);
        picker.selected = current
            .and_then(|id| picker.rows.iter().position(|row| &row.id == id))
            .unwrap_or(0);
        picker
    }

    pub fn edit(&mut self, sessions: &[SessionSummary], generation: u64) {
        self.generation = generation;
        self.content.clear();
        self.search_requested = false;
        self.preview_requested.clear();
        self.selected = 0;
        self.scroll.set(0);
        self.error = None;
        self.searching = !self.query.trim().is_empty();
        self.pending_at = self
            .searching
            .then(|| Instant::now() + Duration::from_millis(150));
        self.rows.clear();
        self.refresh(sessions);
    }

    pub fn refresh(&mut self, sessions: &[SessionSummary]) {
        self.preview_requested.retain(|id, sequence| {
            sessions
                .iter()
                .any(|session| session.id.as_str() == id && session.last_sequence == *sequence)
        });
        let selected = self
            .rows
            .get(self.selected)
            .map(|row| (row.id.clone(), row.content.is_some()));
        let needle = self.query.trim().to_lowercase();
        self.rows = sessions
            .iter()
            .filter(|session| {
                let project = std::path::Path::new(&session.workspace_path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                needle.is_empty()
                    || format!(
                        "{} {} {}",
                        session.id,
                        session.title.as_deref().unwrap_or(""),
                        project
                    )
                    .to_lowercase()
                    .contains(&needle)
            })
            .map(|session| PickerRow {
                id: session.id.clone(),
                content: None,
            })
            .collect();
        let visible: HashSet<_> = sessions.iter().map(|session| &session.id).collect();
        self.rows.extend(
            self.content
                .iter()
                .filter(|row| visible.contains(&row.id))
                .cloned(),
        );
        self.selected = selected
            .and_then(|(id, content)| {
                self.rows
                    .iter()
                    .position(|row| row.id == id && row.content.is_some() == content)
            })
            .unwrap_or(self.selected)
            .min(self.rows.len().saturating_sub(1));
    }

    pub fn batch(
        &mut self,
        generation: u64,
        hits: Vec<blora_storage::SearchMatch>,
        sessions: &[SessionSummary],
    ) {
        if generation != self.generation {
            return;
        }
        for hit in hits {
            if self
                .content
                .iter()
                .any(|row| row.id.as_str() == hit.session_id)
            {
                continue;
            }
            if let Some(session) = sessions
                .iter()
                .find(|session| session.id.as_str() == hit.session_id)
            {
                self.content.push(PickerRow {
                    id: session.id.clone(),
                    content: Some(hit.snippet),
                });
            }
        }
        self.refresh(sessions);
    }

    pub fn ready(&self) -> bool {
        if self.preview_retry_at.is_some_and(|at| Instant::now() < at) && !self.searching {
            return false;
        }
        !self.minimized
            && ((!self.search_requested && self.pending_at.is_some_and(|at| Instant::now() >= at))
                || self
                    .rows
                    .iter()
                    .skip(self.selected.saturating_sub(8))
                    .take(24)
                    .any(|row| !self.preview_requested.contains_key(row.id.as_str())))
    }

    pub fn request(&mut self, sessions: &[SessionSummary]) -> Request {
        let query =
            if !self.search_requested && self.pending_at.is_some_and(|at| Instant::now() >= at) {
                self.search_requested = true;
                Some(self.query.clone())
            } else {
                None
            };
        let ids = self
            .rows
            .iter()
            .skip(self.selected.saturating_sub(8))
            .take(24)
            .filter_map(|row| {
                let session = sessions.iter().find(|session| session.id == row.id)?;
                if self.preview_requested.get(row.id.as_str()) == Some(&session.last_sequence) {
                    return None;
                }
                self.preview_requested
                    .insert(row.id.to_string(), session.last_sequence);
                Some(row.id.clone())
            })
            .collect();
        Request {
            generation: self.generation,
            query,
            ids,
        }
    }

    pub fn apply(&mut self, update: Update, sessions: &[SessionSummary]) {
        match update {
            Update::Batch(generation, hits) => self.batch(generation, hits, sessions),
            Update::Finished(generation, error) if generation == self.generation => {
                self.searching = false;
                self.error = error;
            }
            Update::Previews(generation, rows) if generation == self.generation => {
                for (id, sequence, preview) in rows {
                    self.previews
                        .insert(id.to_string(), (sequence, preview.unwrap_or_default()));
                }
            }
            Update::PreviewFailed(generation, ids) if generation == self.generation => {
                for id in ids {
                    self.preview_requested.remove(id.as_str());
                }
                self.preview_retry_at = Some(Instant::now() + Duration::from_secs(2));
            }
            _ => {}
        }
    }

    pub fn move_selection(&mut self, down: bool) {
        self.selected = if down {
            (self.selected + 1).min(self.rows.len().saturating_sub(1))
        } else {
            self.selected.saturating_sub(1)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_storage::{CreateSession, SqliteStore};
    use blora_types::Mode;

    #[test]
    fn worker_delivers_real_batches_and_cancels_a_full_queue() {
        let store = SqliteStore::open_in_memory().unwrap();
        let id = store
            .create_session(CreateSession {
                title: Some("target".into()),
                workspace_path: "/tmp/project".into(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        for _ in 0..45 {
            store
                .append(blora_events::NewEvent::new(
                    id.clone(),
                    blora_events::KnownPayload::UserInput(blora_events::UserInput {
                        text: "batchneedle 部署".into(),
                    }),
                ))
                .unwrap();
        }
        let runtime = blora_runtime::Runtime::new(store);
        let epoch = std::sync::atomic::AtomicU64::new(1);
        std::thread::scope(|scope| {
            let (tx, rx) = std::sync::mpsc::sync_channel(2);
            let epoch_ref = &epoch;
            let runtime_ref = &runtime;
            let job = scope.spawn(move || {
                work(
                    runtime_ref,
                    Request {
                        generation: 1,
                        query: Some("batchneedle".into()),
                        ids: vec![],
                    },
                    epoch_ref,
                    tx,
                )
            });
            let mut sizes = Vec::new();
            loop {
                match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                    Update::Batch(1, hits) => sizes.push(hits.len()),
                    Update::Finished(1, None) => break,
                    _ => panic!("unexpected worker update"),
                }
            }
            job.join().unwrap();
            assert_eq!(sizes, vec![20, 20, 5]);
            let (tx, _rx) = std::sync::mpsc::sync_channel(1);
            let job = scope.spawn(move || {
                work(
                    runtime_ref,
                    Request {
                        generation: 1,
                        query: Some("batchneedle".into()),
                        ids: vec![],
                    },
                    epoch_ref,
                    tx,
                )
            });
            std::thread::sleep(Duration::from_millis(30));
            epoch.store(2, std::sync::atomic::Ordering::Relaxed);
            job.join().unwrap();
        });
    }

    #[test]
    fn query_lifecycle_and_preview_versions_are_isolated() {
        let store = SqliteStore::open_in_memory().unwrap();
        let id = store
            .create_session(CreateSession {
                title: Some("target".into()),
                workspace_path: "/tmp/project".into(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        let mut sessions = store.list_sessions().unwrap();
        let mut picker = SessionPicker::new(&sessions, Some(&id), 10);
        assert!(picker.ready());
        let request = picker.request(&sessions);
        assert_eq!(request.ids.len(), 1);
        assert!(!picker.ready());
        picker.apply(
            Update::Previews(10, vec![(id.clone(), 0, Some("preview".into()))]),
            &sessions,
        );
        assert_eq!(picker.previews[id.as_str()].1, "preview");
        picker.apply(Update::PreviewFailed(10, vec![id.clone()]), &sessions);
        assert!(!picker.preview_requested.contains_key(id.as_str()));
        assert!(!picker.ready());
        picker.preview_retry_at = None;
        sessions[0].last_sequence += 1;
        picker.refresh(&sessions);
        assert!(picker.ready());
        picker.query = "target".into();
        picker.edit(&sessions, 11);
        picker.apply(Update::Finished(10, Some("old error".into())), &sessions);
        assert!(picker.error.is_none());
        assert!(picker.searching);
        picker.apply(
            Update::Finished(11, Some("current error".into())),
            &sessions,
        );
        assert!(!picker.searching);
        assert_eq!(picker.error.as_deref(), Some("current error"));
        picker.minimized = true;
        assert!(!picker.ready());
        picker.query.clear();
        picker.edit(&sessions, 12);
        assert!(!picker.searching);
        assert!(picker.error.is_none());
    }

    #[test]
    fn staged_results_preserve_selection_and_ignore_old_queries() {
        let store = SqliteStore::open_in_memory().unwrap();
        let a = store
            .create_session(CreateSession {
                title: Some("标题测试".into()),
                workspace_path: "/tmp/project".into(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        let b = store
            .create_session(CreateSession {
                title: Some("other".into()),
                workspace_path: "/tmp/project".into(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        let sessions = store.list_sessions().unwrap();
        let mut picker = SessionPicker::new(&sessions, Some(&a), 1);
        picker.query = "测试".into();
        picker.edit(&sessions, 2);
        assert_eq!(picker.rows.len(), 1);
        assert!(picker.searching);
        let hit = blora_storage::SearchMatch {
            session_id: b.to_string(),
            title: None,
            kind: "user".into(),
            at: String::new(),
            snippet: "内容测试".into(),
        };
        picker.batch(1, vec![hit.clone()], &sessions);
        assert_eq!(picker.rows.len(), 1);
        picker.batch(2, vec![hit.clone(), hit], &sessions);
        assert_eq!(picker.rows.len(), 2);
        assert_eq!(picker.rows[picker.selected].id, a);
        picker.move_selection(true);
        picker.refresh(&sessions);
        assert_eq!(picker.rows[picker.selected].id, b);
        picker.query.clear();
        picker.edit(&sessions, 3);
        assert!(!picker.searching);
        assert!(picker.rows.iter().all(|row| row.content.is_none()));
    }
}

pub struct Request {
    generation: u64,
    query: Option<String>,
    ids: Vec<SessionId>,
}

pub enum Update {
    PreviewFailed(u64, Vec<SessionId>),
    Batch(u64, Vec<blora_storage::SearchMatch>),
    Finished(u64, Option<String>),
    Previews(u64, Vec<(SessionId, u64, Option<String>)>),
}

pub fn work(
    runtime: &blora_runtime::Runtime,
    request: Request,
    epoch: &std::sync::atomic::AtomicU64,
    tx: std::sync::mpsc::SyncSender<Update>,
) {
    let send = |mut update| {
        loop {
            if epoch.load(std::sync::atomic::Ordering::Relaxed) != request.generation {
                return false;
            }
            match tx.try_send(update) {
                Ok(()) => return true,
                Err(std::sync::mpsc::TrySendError::Full(value)) => {
                    update = value;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return false,
            }
        }
    };
    let current = || epoch.load(std::sync::atomic::Ordering::Relaxed) == request.generation;
    if !current() {
        return;
    }
    if !request.ids.is_empty() {
        match runtime.session_message_previews(&request.ids) {
            Ok(previews) if current() => {
                let _ = send(Update::Previews(request.generation, previews));
            }
            Err(_) if current() => {
                let _ = send(Update::PreviewFailed(
                    request.generation,
                    request.ids.clone(),
                ));
            }
            _ => {}
        }
    }
    let Some(query) = request.query else {
        return;
    };
    let mut cursor = None;
    while current() {
        match runtime.session_search_page(&query, 20, cursor.as_ref()) {
            Ok((hits, next)) => {
                if !current() {
                    return;
                }
                if !send(Update::Batch(request.generation, hits)) {
                    return;
                }
                if next.is_none() {
                    let _ = send(Update::Finished(request.generation, None));
                    return;
                }
                cursor = next;
                std::thread::yield_now();
            }
            Err(error) => {
                let _ = send(Update::Finished(
                    request.generation,
                    Some(error.to_string()),
                ));
                return;
            }
        }
    }
}

pub struct CancelOnDrop<'a>(pub &'a std::sync::atomic::AtomicU64);
impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
    }
}
