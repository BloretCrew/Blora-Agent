// SPDX-License-Identifier: GPL-3.0-or-later
use super::editor::Editor;
use super::*;

#[derive(Clone, Debug)]
pub(super) enum Leave {
    Back,
    Refresh,
    Close,
    Project(PathBuf),
    CloseDocument(PathBuf),
}

pub(super) struct OpenDocument {
    pub(super) path: PathBuf,
    pub(super) root: PathBuf,
    pub(super) editor: Option<Editor>,
    pub(super) preview: String,
    pub(super) reason: String,
    pub(super) scroll: usize,
    pub(super) horizontal: usize,
}
pub(super) struct FileComparison {
    pub(super) lines: Vec<String>,
    pub(super) scroll: usize,
    disk: Option<Editor>,
    preview: String,
    path: PathBuf,
}

#[derive(Debug)]
pub(super) struct FileEntry {
    pub(super) path: PathBuf,
    pub(super) directory: bool,
}

mod render;
mod worker;

pub(super) use worker::*;
pub(super) struct Files {
    pub(super) documents: Vec<OpenDocument>,
    pub(super) active_document: Option<usize>,
    pub(super) editor: Option<Editor>,
    pub(super) pending_leave: Option<Leave>,
    pub(super) blocked_project: Option<PathBuf>,
    pub(super) root: PathBuf,
    pub(super) directory: PathBuf,
    pub(super) generation: u64,
    pub(super) entries: Vec<FileEntry>,
    expanded: std::collections::BTreeSet<PathBuf>,
    pending_tree: Option<PathBuf>,
    pending_compare: bool,
    pub(super) comparison: Option<FileComparison>,
    pub(super) selected: usize,
    pub(super) preview: Option<String>,
    pub(super) preview_path: Option<PathBuf>,
    pub(super) horizontal: usize,
    pub(super) max_scroll: std::cell::Cell<usize>,
    pub(super) max_horizontal: std::cell::Cell<usize>,
    positions: std::collections::HashMap<PathBuf, (usize, usize)>,
    restore: Option<PathBuf>,
    pub(super) message: String,
    pub(super) worker: std::sync::Arc<FileWorker>,
    pub(super) handle: Option<thread::JoinHandle<()>>,
    pub(super) cancel: CancelToken,
    pub(super) scroll: usize,
}
impl Drop for Files {
    fn drop(&mut self) {
        self.cancel();
        {
            let mut work = self.worker.work.lock().unwrap();
            work.stop = true;
        }
        self.worker.ready.notify_one();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl Files {
    pub(super) fn cycle_document(&mut self, backwards: bool) {
        if self.documents.is_empty() {
            return;
        }
        let n = self.documents.len();
        let i = self.active_document.unwrap_or(0);
        self.activate_document(if backwards {
            (i + n - 1) % n
        } else {
            (i + 1) % n
        });
    }
    pub(super) fn new() -> Self {
        let worker = std::sync::Arc::new(FileWorker::default());
        let background = worker.clone();
        let handle = thread::Builder::new()
            .name("blora-right-files".into())
            .spawn(move || background.run())
            .ok();
        Self {
            documents: vec![],
            active_document: None,
            editor: None,
            pending_leave: None,
            blocked_project: None,
            root: PathBuf::new(),
            directory: PathBuf::new(),
            generation: 0,
            entries: vec![],
            expanded: Default::default(),
            pending_tree: None,
            pending_compare: false,
            comparison: None,
            selected: 0,
            preview: None,
            preview_path: None,
            horizontal: 0,
            max_scroll: 0.into(),
            max_horizontal: 0.into(),
            positions: Default::default(),
            restore: None,
            message: String::new(),
            worker,
            handle,
            cancel: CancelToken::new(),
            scroll: 0,
        }
    }
    pub(super) fn load(&mut self, path: PathBuf, preview: bool) {
        self.capture_active_document();
        self.active_document = None;
        self.cancel();
        self.message = i18n::tr("right.loading", "加载中…");
        self.preview = None;
        self.editor = None;
        if !preview && !self.directory.as_os_str().is_empty() {
            self.positions
                .insert(self.directory.clone(), (self.selected, self.scroll));
        }
        self.scroll = 0;
        self.horizontal = 0;
        self.preview_path = preview.then(|| path.clone());
        if !preview {
            self.expanded.clear();
            self.selected = 0;
            self.directory = path.clone();
            self.entries.clear();
            if let Some((selected, scroll)) = self.positions.get(&self.directory) {
                self.selected = *selected;
                self.scroll = *scroll;
            }
        }
        if self.handle.is_none() {
            self.message = "could not start file worker".to_owned();
            return;
        }
        self.cancel = CancelToken::new();
        let mut work = self.worker.work.lock().unwrap();
        work.pending = Some(FileRequest {
            generation: self.generation,
            root: self.root.clone(),
            path,
            preview,
            cancel: self.cancel.clone(),
        });
        self.worker.ready.notify_one();
    }
    pub(super) fn cancel(&mut self) {
        self.pending_tree = None;
        self.pending_compare = false;
        self.comparison = None;
        self.cancel.cancel();
        self.generation = self.generation.wrapping_add(1);
        let mut work = self.worker.work.lock().unwrap();
        work.pending = None;
        work.result = None;
    }
    pub(super) fn close(&mut self) {
        self.editor = None;
        self.documents.clear();
        self.active_document = None;
        self.pending_leave = None;
        self.blocked_project = None;
        self.cancel();
        self.root.clear();
        self.directory.clear();
        self.entries.clear();
        self.expanded.clear();
        self.preview = None;
        self.preview_path = None;
        self.positions.clear();
        self.restore = None;
        self.message.clear();
    }

    pub(super) fn sync(&mut self, root: &Path) {
        if self.root != root {
            if self
                .documents
                .iter()
                .any(|d| d.editor.as_ref().is_some_and(Editor::dirty))
                || self.dirty()
            {
                if self.blocked_project.as_deref() != Some(root) && self.pending_leave.is_none() {
                    self.pending_leave = Some(Leave::Project(root.to_path_buf()));
                }
                return;
            }
            self.blocked_project = None;
            self.positions.clear();
            self.documents.clear();
            self.active_document = None;
            self.restore = None;
            self.directory.clear();
            self.root = root.to_path_buf();
            self.load(root.to_path_buf(), false);
        }
        let update = self.worker.work.lock().unwrap().result.take();
        if let Some(update) = update {
            if update.generation != self.generation {
                return;
            }
            if self.pending_compare {
                self.pending_compare = false;
                match update.result {
                    Ok(FileData::Document {
                        preview,
                        editor,
                        reason,
                    }) => {
                        let draft = self
                            .editor
                            .as_ref()
                            .map(|editor| editor.text())
                            .unwrap_or("");
                        self.comparison = Some(FileComparison {
                            lines: comparison_lines(
                                draft,
                                editor.as_ref().map(Editor::text).unwrap_or(&preview),
                            ),
                            scroll: 0,
                            disk: editor,
                            preview: preview.clone(),
                            path: self.preview_path.clone().unwrap_or_default(),
                        });
                        self.message = reason;
                    }
                    Err(error) => self.message = error,
                    _ => {}
                }
                return;
            }
            if let Some(parent) = self.pending_tree.take() {
                match update.result {
                    Ok(FileData::List(children)) => {
                        if let Some(index) =
                            self.entries.iter().position(|entry| entry.path == parent)
                        {
                            self.expanded.insert(parent);
                            self.entries.splice(index + 1..index + 1, children);
                        }
                    }
                    Err(error) => self.message = error,
                    _ => {}
                }
                return;
            }
            match update.result {
                Ok(FileData::List(entries)) => {
                    self.entries = entries;
                    if let Some(path) = self.restore.take() {
                        if let Some(index) =
                            self.entries.iter().position(|entry| entry.path == path)
                        {
                            self.selected = index;
                        }
                    }
                    self.selected = self.selected.min(self.entries.len().saturating_sub(1));
                    self.message = if self.entries.is_empty() {
                        i18n::tr("right.empty", "空目录")
                    } else {
                        String::new()
                    };
                }
                Ok(FileData::Document {
                    preview,
                    editor,
                    reason,
                }) => {
                    self.preview = Some(preview);
                    self.editor = editor;
                    self.message = reason;
                    if let Some(path) = self.preview_path.clone() {
                        self.register_active_document(path);
                    }
                }
                Ok(FileData::Preview(text)) => {
                    self.preview = Some(text);
                    self.message = String::new();
                }
                Err(error) => {
                    self.preview = None;
                    self.entries.clear();
                    self.selected = 0;
                    self.message = error;
                }
            }
        }
    }
    pub(super) fn open(&mut self, index: usize) {
        if let Some(entry) = self.entries.get(index) {
            let path = entry.path.clone();
            let preview = !entry.directory;
            self.selected = index;
            if preview
                && let Some(existing) = self
                    .documents
                    .iter()
                    .position(|doc| doc.path == path && doc.root == self.root)
            {
                self.activate_document(existing);
                return;
            }
            self.load(path, preview);
        }
    }
    // Explorer expansion uses the existing bounded background reader, while
    // leaving the active document, cursor, undo history and viewport intact.
    pub(super) fn open_in_explorer(&mut self, index: usize) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        if !entry.directory {
            self.open(index);
            return;
        }
        let path = entry.path.clone();
        self.selected = index;
        self.cancel();
        if self.expanded.remove(&path) {
            self.entries
                .retain(|entry| entry.path == path || !entry.path.starts_with(&path));
            self.expanded
                .retain(|expanded| !expanded.starts_with(&path));
            return;
        }
        if self.handle.is_none() {
            self.message = "could not start file worker".to_owned();
            return;
        }
        self.pending_tree = Some(path.clone());
        self.cancel = CancelToken::new();
        let mut work = self.worker.work.lock().unwrap();
        work.pending = Some(FileRequest {
            generation: self.generation,
            root: self.root.clone(),
            path,
            preview: false,
            cancel: self.cancel.clone(),
        });
        self.worker.ready.notify_one();
    }
    pub(super) fn tree_horizontal(&mut self, expand: bool) {
        let Some(entry) = self.entries.get(self.selected) else {
            return;
        };
        let path = entry.path.clone();
        let expanded = self.expanded.contains(&path);
        if entry.directory && expanded != expand {
            self.open_in_explorer(self.selected);
        } else if expand && expanded {
            if self
                .entries
                .get(self.selected + 1)
                .is_some_and(|entry| entry.path.starts_with(&path))
            {
                self.selected += 1;
            }
        } else if !expand {
            if let Some(parent) = path
                .parent()
                .and_then(|parent| self.entries.iter().position(|entry| entry.path == parent))
            {
                self.selected = parent;
            }
        }
    }
    pub(super) fn capture_active_document(&mut self) {
        let Some(index) = self.active_document else {
            return;
        };
        if let Some(document) = self.documents.get_mut(index) {
            document.editor = self.editor.as_ref().map(Editor::clone_state);
            document.preview = self.preview.clone().unwrap_or_default();
            document.reason = self.message.clone();
            document.scroll = self.scroll;
            document.horizontal = self.horizontal;
        }
    }
    pub(super) fn activate_document(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        self.capture_active_document();
        self.cancel();
        let document = &mut self.documents[index];
        self.root = document.root.clone();
        self.preview_path = Some(document.path.clone());
        self.editor = document.editor.as_ref().map(Editor::clone_state);
        self.preview = Some(document.preview.clone());
        self.message = document.reason.clone();
        self.scroll = document.scroll;
        self.horizontal = document.horizontal;
        self.active_document = Some(index);
    }
    pub(super) fn register_active_document(&mut self, path: PathBuf) {
        if !self
            .preview_path
            .as_ref()
            .is_some_and(|active| active == &path)
        {
            return;
        }
        if let Some(index) = self
            .documents
            .iter()
            .position(|doc| doc.path == path && doc.root == self.root)
        {
            self.active_document = Some(index);
            self.capture_active_document();
        } else {
            self.documents.push(OpenDocument {
                path: path.clone(),
                root: self.root.clone(),
                editor: self.editor.as_ref().map(Editor::clone_state),
                preview: self.preview.clone().unwrap_or_default(),
                reason: self.message.clone(),
                scroll: self.scroll,
                horizontal: self.horizontal,
            });
            self.active_document = Some(self.documents.len() - 1);
        }
    }
    pub(super) fn activate_path(&mut self, path: &Path) -> bool {
        let Some(index) = self
            .documents
            .iter()
            .position(|d| d.path == path && d.root == self.root)
        else {
            return false;
        };
        self.activate_document(index);
        true
    }
    pub(super) fn close_document(&mut self, index: usize) -> bool {
        self.capture_active_document();
        self.remove_document(index, true)
    }
    fn remove_document(&mut self, index: usize, guard_dirty: bool) -> bool {
        if index >= self.documents.len() {
            return false;
        }
        let active = self.active_document == Some(index);
        if guard_dirty
            && self.documents[index]
                .editor
                .as_ref()
                .is_some_and(Editor::dirty)
        {
            return false;
        }
        self.documents.remove(index);
        self.active_document = match self.active_document {
            Some(current) if current == index => None,
            Some(current) if current > index => Some(current - 1),
            other => other,
        };
        if active {
            self.editor = None;
            self.preview = None;
            self.preview_path = None;
            self.active_document = if self.documents.is_empty() {
                None
            } else {
                Some(index.min(self.documents.len() - 1))
            };
            if let Some(next) = self.active_document {
                self.activate_document(next);
            }
        }
        true
    }
    pub(super) fn dirty(&self) -> bool {
        self.editor.as_ref().is_some_and(Editor::dirty)
    }
    pub(super) fn save(&mut self) -> bool {
        if let Some(editor) = &mut self.editor {
            match editor.save(&self.root) {
                Ok(()) => {
                    self.message = i18n::tr("right.file_saved", "已保存");
                    return true;
                }
                Err(error) => {
                    self.compare_disk();
                    self.message = error;
                }
            }
        }
        false
    }
    pub(super) fn compare_disk(&mut self) {
        let Some(path) = self.preview_path.clone() else {
            return;
        };
        self.cancel();
        if self.handle.is_none() {
            return;
        }
        self.pending_compare = true;
        self.cancel = CancelToken::new();
        let mut work = self.worker.work.lock().unwrap();
        work.pending = Some(FileRequest {
            generation: self.generation,
            root: self.root.clone(),
            path,
            preview: true,
            cancel: self.cancel.clone(),
        });
        self.worker.ready.notify_one();
    }
    pub(super) fn reload_compared(&mut self) {
        let Some(comparison) = self.comparison.as_ref() else {
            return;
        };
        if comparison.path != self.preview_path.clone().unwrap_or_default() {
            return;
        }
        let Some(disk) = comparison.disk.as_ref() else {
            self.message = "磁盘内容不可编辑，保留草稿".into();
            return;
        };
        if let Err(error) = disk.check(&self.root) {
            self.message = error;
            self.compare_disk();
            return;
        }
        let comparison = self.comparison.take().unwrap();
        self.editor = comparison.disk;
        self.pending_leave = None;
        self.preview = Some(comparison.preview);
        self.scroll = 0;
        self.horizontal = 0;
        self.message = "已重新加载磁盘内容".into();
        self.capture_active_document();
    }
    pub(super) fn confirm(&mut self, save: bool) -> Option<Leave> {
        self.capture_active_document();
        if save {
            let project_leave = matches!(self.pending_leave, Some(Leave::Project(_)));
            if project_leave {
                for document in &mut self.documents {
                    if document.editor.as_ref().is_some_and(Editor::dirty) {
                        let Some(editor) = document.editor.as_mut() else {
                            return None;
                        };
                        if let Err(error) = editor.save(&document.root) {
                            self.message = error;
                            return None;
                        }
                    }
                }
            } else if let Some(Leave::CloseDocument(path)) = &self.pending_leave {
                if let Some(document) = self.documents.iter_mut().find(|d| &d.path == path) {
                    if let Some(editor) = document.editor.as_mut() {
                        if let Err(error) = editor.save(&document.root) {
                            self.message = error;
                            return None;
                        }
                    }
                }
            } else if !self.save() {
                return None;
            }
        }
        let leave = self.pending_leave.take()?;
        match &leave {
            Leave::CloseDocument(path) => {
                if let Some(index) = self.documents.iter().position(|d| &d.path == path) {
                    self.remove_document(index, save);
                }
            }
            Leave::Project(_) => {
                self.documents.clear();
                self.active_document = None;
                self.editor = None;
            }
            _ => {
                self.capture_active_document();
                self.editor = None;
            }
        }
        Some(leave)
    }
    pub(super) fn cancel_leave(&mut self) {
        if let Some(Leave::Project(root)) = self.pending_leave.take() {
            self.blocked_project = Some(root);
        }
    }
    pub(super) fn refresh(&mut self) {
        if self.dirty() {
            self.pending_leave = Some(Leave::Refresh);
            return;
        }
        let path = self
            .preview_path
            .clone()
            .unwrap_or_else(|| self.directory.clone());
        let preview = self.preview_path.is_some();
        self.restore = self
            .entries
            .get(self.selected)
            .map(|entry| entry.path.clone());
        self.load(path, preview);
    }
    pub(super) fn back(&mut self) {
        if self.dirty() {
            self.pending_leave = Some(Leave::Back);
            return;
        }
        self.capture_active_document();
        self.active_document = None;
        self.editor = None;
        self.cancel();
        self.message.clear();
        if self.preview_path.take().is_some() {
            self.preview = None;
            self.scroll = 0;
            self.horizontal = 0;
            if self.entries.is_empty() {
                self.load(self.directory.clone(), false);
            }
        } else if self.directory != self.root {
            let child = self.directory.clone();
            if let Some(parent) = child.parent() {
                self.restore = Some(child.clone());
                self.load(parent.to_path_buf(), false);
            }
        }
    }
    pub(super) fn show_explorer(&mut self) {
        self.capture_active_document();
        self.active_document = None;
        self.editor = None;
        self.preview = None;
        self.preview_path = None;
        self.scroll = 0;
        self.horizontal = 0;
        self.message.clear();
        self.load(self.directory.clone(), false);
    }
}
#[cfg(test)]
mod editor_tests {
    use super::*;
    use std::fs;
    #[test]
    fn read_errors_identify_path_and_permission_recovery() {
        let path = Path::new("/project/private");
        let denied = file_read_error(path, std::io::ErrorKind::PermissionDenied.into());
        assert!(denied.contains(&path.display().to_string()));
        assert!(denied.contains(&i18n::tr("right.file_access_denied", "没有权限读取此路径")));
        assert!(denied.contains(&i18n::tr(
            "right.file_access_help",
            "请检查目录权限及终端应用的文件访问授权，恢复访问后点击刷新（r）。",
        )));
        let missing = file_read_error(path, std::io::ErrorKind::NotFound.into());
        assert!(missing.contains(&path.display().to_string()));
        assert!(missing.contains(&i18n::tr("right.file_read_failed", "无法读取此路径")));
        let root = tempfile::tempdir().unwrap();
        let missing_path = root.path().join("missing");
        let error =
            read_file_data_cancelled(root.path(), &missing_path, false, &CancelToken::new())
                .err()
                .unwrap();
        assert!(error.contains(&missing_path.display().to_string()));
    }
    #[test]
    fn external_changes_can_be_compared_kept_and_reloaded_without_stale_overwrite() {
        let (root, mut files) = opened(b"original\n");
        let path = root.path().join("example.rs");
        files.register_active_document(path.clone());
        files.editor.as_mut().unwrap().insert("draft ");
        fs::write(&path, "external\n").unwrap();
        assert!(!files.save());
        let wait = |files: &mut Files| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while files.pending_compare {
                files.sync(root.path());
                assert!(std::time::Instant::now() < deadline);
                thread::sleep(std::time::Duration::from_millis(2));
            }
        };
        wait(&mut files);
        let comparison = files.comparison.as_ref().unwrap();
        assert!(
            comparison
                .lines
                .iter()
                .any(|line| line == "-draft original")
        );
        assert!(comparison.lines.iter().any(|line| line == "+external"));
        assert_eq!(files.editor.as_ref().unwrap().text(), "draft original\n");
        files.comparison = None;
        assert!(files.dirty());
        files.compare_disk();
        wait(&mut files);
        fs::write(&path, "newer\n").unwrap();
        files.reload_compared();
        assert!(files.pending_compare);
        assert!(files.dirty());
        wait(&mut files);
        files.reload_compared();
        assert_eq!(files.editor.as_ref().unwrap().text(), "newer\n");
        assert!(!files.dirty());
        assert_eq!(fs::read_to_string(&path).unwrap(), "newer\n");
        assert_eq!(
            files.documents[0].editor.as_ref().unwrap().text(),
            "newer\n"
        );
        assert!(
            comparison_lines("a", "a\n")
                .iter()
                .any(|line| line.contains("末尾换行"))
        );
    }
    #[test]
    fn find_replace_layout_keeps_tabs_code_and_utf8_input_cursor_visible() {
        use ratatui::{Terminal, backend::TestBackend};
        let (_root, mut files) = opened("中文 match match\nsecond".as_bytes());
        files.register_active_document(files.preview_path.clone().unwrap());
        let mut panel = RightPanel::default();
        panel.files = files;
        panel.focused = true;
        let editor = panel.files.editor.as_mut().unwrap();
        editor.find_open(true);
        for ch in "match".chars() {
            editor.find_key(crossterm::event::KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::NONE,
            ));
        }
        editor.find_key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        ));
        for ch in "很长的中文替换内容🙂".repeat(5).chars() {
            editor.find_key(crossterm::event::KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::NONE,
            ));
        }
        for width in [12, 24, 44, 100] {
            let mut terminal = Terminal::new(TestBackend::new(width, 18)).unwrap();
            let mut hits = vec![];
            let mut content = None;
            terminal
                .draw(|frame| {
                    content =
                        panel.draw_files(frame, None, Rect::new(0, 0, width, 18), &mut hits, true);
                })
                .unwrap();
            let code = content.unwrap();
            assert_eq!(code.y, 4);
            let cursor = terminal.get_cursor_position().unwrap();
            assert_eq!(cursor.y, 3);
            assert!(cursor.x < width);
            assert!(
                hits.iter()
                    .any(|(_, action)| matches!(action, Action::Document(_)))
            );
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(code.x, 2)].symbol(), "找");
            assert_eq!(buffer[(code.x, 3)].symbol(), "替");
            if width >= 44 {
                assert_ne!(
                    buffer[(code.x + 7, code.y)].bg,
                    ratatui::style::Color::Reset
                );
                assert_ne!(
                    buffer[(code.x + 13, code.y)].bg,
                    ratatui::style::Color::Reset
                );
            }
        }
    }
    fn opened(text: &[u8]) -> (tempfile::TempDir, Files) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("example.rs");
        fs::write(&path, text).unwrap();
        let FileData::Document {
            preview,
            editor,
            reason,
        } = read_file_data_cancelled(root.path(), &path, true, &CancelToken::new()).unwrap()
        else {
            panic!("expected document")
        };
        let mut files = Files::new();
        files.root = root.path().into();
        files.directory = root.path().into();
        files.preview_path = Some(path);
        files.preview = Some(preview);
        files.editor = editor;
        files.message = reason;
        (root, files)
    }
    #[test]
    fn tree_expansion_preserves_document_and_cancels_stale_children() {
        let (root, mut files) = opened(b"hello");
        let path = root.path().join("example.rs");
        files.register_active_document(path.clone());
        files.editor.as_mut().unwrap().insert("draft");
        files.scroll = 5;
        let folder = root.path().join("folder");
        let nested = folder.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("child.rs"), "child").unwrap();
        files.entries = vec![FileEntry {
            path: folder.clone(),
            directory: true,
        }];
        let wait = |files: &mut Files| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while files.pending_tree.is_some() {
                files.sync(root.path());
                assert!(
                    std::time::Instant::now() < deadline,
                    "tree reader timed out"
                );
                thread::sleep(std::time::Duration::from_millis(2));
            }
        };
        files.open_in_explorer(0);
        wait(&mut files);
        assert_eq!(files.entries[1].path, nested);
        assert!(files.expanded.contains(&folder));
        files.open_in_explorer(1);
        wait(&mut files);
        assert_eq!(files.entries[2].path, nested.join("child.rs"));
        files.selected = 0;
        files.tree_horizontal(true);
        assert_eq!(files.selected, 1);
        files.tree_horizontal(true);
        assert_eq!(files.selected, 2);
        files.tree_horizontal(false);
        assert_eq!(files.selected, 1);
        files.tree_horizontal(false);
        assert_eq!(files.entries.len(), 2);
        files.tree_horizontal(false);
        assert_eq!(files.selected, 0);
        files.open_in_explorer(0);
        assert_eq!(files.entries.len(), 1);
        assert!(files.expanded.is_empty());
        files.open_in_explorer(0);
        wait(&mut files);
        files.open_in_explorer(1);
        files.open_in_explorer(0);
        files.sync(root.path());
        assert_eq!(files.entries.len(), 1);
        assert!(files.pending_tree.is_none());
        assert_eq!(files.preview_path.as_ref(), Some(&path));
        assert_eq!(files.active_document, Some(0));
        assert_eq!(files.scroll, 5);
        assert_eq!(files.editor.as_ref().unwrap().text(), "drafthello");
        files
            .editor
            .as_mut()
            .unwrap()
            .key(crossterm::event::KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL,
            ));
        assert_eq!(files.editor.as_ref().unwrap().text(), "hello");
    }
    #[test]
    fn only_complete_utf8_text_is_editable() {
        for bytes in [vec![b'a'; 65537], vec![0, 1], vec![0xff, b'a']] {
            let (_root, files) = opened(&bytes);
            assert!(files.editor.is_none());
            assert!(files.message.contains("只读"));
        }
        let (_root, files) = opened(b"");
        assert_eq!(files.editor.as_ref().unwrap().text(), "");
    }
    #[test]
    fn loading_another_file_preserves_active_draft_and_undo() {
        let (root, mut files) = opened(b"hello");
        let path = root.path().join("example.rs");
        files.register_active_document(path.clone());
        files.editor.as_mut().unwrap().insert("draft");
        files.load(root.path().join("other.rs"), true);
        assert!(files.activate_path(&path));
        assert_eq!(files.editor.as_ref().unwrap().text(), "drafthello");
        files
            .editor
            .as_mut()
            .unwrap()
            .key(crossterm::event::KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL,
            ));
        assert_eq!(files.editor.as_ref().unwrap().text(), "hello");
    }
    #[test]
    fn explorer_keeps_dirty_document_and_last_tab_closes_safely() {
        let (root, mut files) = opened(b"hello");
        let path = root.path().join("example.rs");
        files.register_active_document(path.clone());
        files.editor.as_mut().unwrap().insert("draft");
        files.show_explorer();
        assert!(files.pending_leave.is_none());
        assert!(files.activate_path(&path));
        assert_eq!(files.editor.as_ref().unwrap().text(), "drafthello");
        files.pending_leave = Some(Leave::CloseDocument(path));
        assert!(files.confirm(false).is_some());
        assert!(files.documents.is_empty());
        assert!(files.active_document.is_none());
    }
    #[test]
    fn closing_dirty_documents_requires_confirmation_and_keeps_other_tabs() {
        let (root, mut files) = opened(b"first");
        let first = root.path().join("example.rs");
        files.register_active_document(first.clone());
        files.editor.as_mut().unwrap().insert("draft ");
        files.capture_active_document();

        let second = root.path().join("second.rs");
        fs::write(&second, "second").unwrap();
        let FileData::Document {
            preview,
            editor,
            reason,
        } = read_file_data_cancelled(root.path(), &second, true, &CancelToken::new()).unwrap()
        else {
            panic!("expected document")
        };
        files.preview_path = Some(second.clone());
        files.preview = Some(preview);
        files.editor = editor;
        files.message = reason;
        files.register_active_document(second.clone());

        assert_eq!(files.documents.len(), 2);
        assert!(files.documents[0].editor.as_ref().unwrap().dirty());
        assert!(!files.close_document(0));
        files.pending_leave = Some(Leave::CloseDocument(first.clone()));
        assert!(matches!(files.confirm(true), Some(Leave::CloseDocument(_))));
        assert_eq!(files.documents.len(), 1);
        assert_eq!(files.active_document, Some(0));
        assert_eq!(files.preview_path.as_ref(), Some(&second));
        assert_eq!(fs::read_to_string(first).unwrap(), "draft first");
    }

    #[test]
    fn back_cancel_and_project_change_preserve_draft() {
        let (root, mut files) = opened(b"hello");
        files.editor.as_mut().unwrap().insert("draft");
        files.back();
        assert!(files.pending_leave.is_some());
        assert!(files.dirty());
        files.cancel_leave();
        assert!(files.pending_leave.is_none());
        assert!(files.dirty());
        let next = tempfile::tempdir().unwrap();
        files.sync(next.path());
        assert_eq!(files.root, root.path());
        assert!(matches!(files.pending_leave, Some(Leave::Project(_))));
        files.cancel_leave();
        files.sync(next.path());
        assert!(files.pending_leave.is_none());
        assert!(files.dirty());
    }
    #[test]
    fn failed_save_keeps_confirmation_and_draft() {
        let (root, mut files) = opened(b"hello");
        files.editor.as_mut().unwrap().insert("draft");
        files.pending_leave = Some(Leave::Close);
        fs::write(root.path().join("example.rs"), "external").unwrap();
        assert!(files.confirm(true).is_none());
        assert!(files.pending_leave.is_some());
        assert!(files.dirty());
        assert!(matches!(files.confirm(false), Some(Leave::Close)));
        assert_eq!(
            fs::read_to_string(root.path().join("example.rs")).unwrap(),
            "external"
        );
    }
}
