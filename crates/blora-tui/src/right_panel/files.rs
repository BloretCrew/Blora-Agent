// SPDX-License-Identifier: GPL-3.0-or-later
use super::editor::Editor;
use super::*;
use ratatui::style::Modifier;

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
pub(super) struct FileResult {
    pub(super) generation: u64,
    pub(super) result: Result<FileData, String>,
}
pub(super) enum FileData {
    List(Vec<FileEntry>),
    Preview(String),
    Document {
        preview: String,
        editor: Option<Editor>,
        reason: String,
    },
}
pub(super) struct FileRequest {
    pub(super) generation: u64,
    pub(super) root: PathBuf,
    pub(super) path: PathBuf,
    pub(super) preview: bool,
    pub(super) cancel: CancelToken,
}
#[derive(Default)]
pub(super) struct FileWork {
    pub(super) pending: Option<FileRequest>,
    pub(super) result: Option<FileResult>,
    pub(super) stop: bool,
}
#[derive(Default)]
pub(super) struct FileWorker {
    pub(super) work: std::sync::Mutex<FileWork>,
    pub(super) ready: std::sync::Condvar,
}
impl FileWorker {
    pub(super) fn run(&self) {
        loop {
            let request = {
                let mut work = self.work.lock().unwrap();
                while work.pending.is_none() && !work.stop {
                    work = self.ready.wait(work).unwrap();
                }
                if work.stop {
                    return;
                }
                work.pending.take().unwrap()
            };
            let result = read_file_data_cancelled(
                &request.root,
                &request.path,
                request.preview,
                &request.cancel,
            );
            let mut work = self.work.lock().unwrap();
            if !work.stop && !request.cancel.is_cancelled() {
                work.result = Some(FileResult {
                    generation: request.generation,
                    result,
                });
            }
        }
    }
}
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
                            .map(|editor| editor.text.as_str())
                            .unwrap_or("");
                        self.comparison = Some(FileComparison {
                            lines: comparison_lines(
                                draft,
                                editor
                                    .as_ref()
                                    .map(|editor| editor.text.as_str())
                                    .unwrap_or(&preview),
                            ),
                            scroll: 0,
                            disk: editor,
                            preview,
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
pub(super) fn read_file_data(root: &Path, path: &Path, preview: bool) -> Result<FileData, String> {
    match read_file_data_cancelled(root, path, preview, &CancelToken::new())? {
        FileData::Document { reason, .. }
            if reason == i18n::tr("right.file_binary", "只读 · 二进制文件") =>
        {
            Err(reason)
        }
        FileData::Document { preview, .. } => Ok(FileData::Preview(preview)),
        data => Ok(data),
    }
}
pub(super) fn file_cancelled(cancel: &CancelToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("file request cancelled".to_owned())
    } else {
        Ok(())
    }
}
fn file_read_error(path: &Path, error: std::io::Error) -> String {
    let heading = if error.kind() == std::io::ErrorKind::PermissionDenied {
        i18n::tr("right.file_access_denied", "没有权限读取此路径")
    } else {
        i18n::tr("right.file_read_failed", "无法读取此路径")
    };
    let mut message = format!("{heading}\n{}\n{error}", path.display());
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        message.push_str(&format!(
            "\n{}",
            i18n::tr(
                "right.file_access_help",
                "请检查目录权限及终端应用的文件访问授权，恢复访问后点击刷新（r）。",
            )
        ));
    }
    message
}

pub(super) fn read_file_data_cancelled(
    root: &Path,
    path: &Path,
    preview: bool,
    cancel: &CancelToken,
) -> Result<FileData, String> {
    file_cancelled(cancel)?;
    let root = root.canonicalize().map_err(|e| file_read_error(root, e))?;
    file_cancelled(cancel)?;
    let requested_path = path;
    let path = path.canonicalize().map_err(|e| file_read_error(path, e))?;
    file_cancelled(cancel)?;
    if !path.starts_with(&root) {
        return Err(i18n::tr("right.boundary", "路径超出项目边界"));
    }
    if preview {
        if !path.is_file() {
            return Err(i18n::tr("right.special", "仅支持预览普通文件"));
        }
        file_cancelled(cancel)?;
        let mut file = fs::File::open(&path).map_err(|e| file_read_error(&path, e))?;
        let metadata = file.metadata().map_err(|e| file_read_error(&path, e))?;
        let truncated = metadata.len() > 65536;
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        while bytes.len() < 65536 {
            file_cancelled(cancel)?;
            let remaining = (65536 - bytes.len()).min(chunk.len());
            let count = file
                .read(&mut chunk[..remaining])
                .map_err(|e| file_read_error(&path, e))?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        file_cancelled(cancel)?;
        let valid = std::str::from_utf8(&bytes).is_ok();
        let binary = bytes
            .iter()
            .any(|b| *b == 0 || (*b < 32 && !matches!(*b, 9 | 10 | 13)));
        let reason = if truncated {
            i18n::tr("right.file_truncated", "只读 · 仅显示前 64 KiB")
        } else if binary {
            i18n::tr("right.file_binary", "只读 · 二进制文件")
        } else if !valid {
            i18n::tr("right.file_invalid_utf8", "只读 · 无效 UTF-8")
        } else {
            String::new()
        };
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if binary {
            text = text
                .chars()
                .map(|c| {
                    if c.is_control() && !matches!(c, '\n' | '\r' | '\t') {
                        '�'
                    } else {
                        c
                    }
                })
                .collect();
        }
        let editor = if reason.is_empty() {
            Some(Editor::new(requested_path, bytes, metadata)?)
        } else {
            None
        };
        if truncated {
            text.push_str(&format!(
                "\n{}",
                i18n::tr("right.file_truncated_note", "仅显示前 64 KiB")
            ));
        }
        if text.is_empty() {
            text = i18n::tr("right.empty_file", "空文件");
        }
        Ok(FileData::Document {
            preview: text,
            editor,
            reason: reason.into(),
        })
    } else {
        let mut entries = Vec::new();
        file_cancelled(cancel)?;
        let mut directory = fs::read_dir(&path).map_err(|e| file_read_error(&path, e))?;
        for _ in 0..1001 {
            file_cancelled(cancel)?;
            let Some(entry) = directory.next() else {
                break;
            };
            file_cancelled(cancel)?;
            let entry = entry.map_err(|e| file_read_error(&path, e))?;
            let canonical = match entry.path().canonicalize() {
                Ok(p) if p.starts_with(&root) => p,
                _ => continue,
            };
            file_cancelled(cancel)?;
            if !canonical.is_dir() && !canonical.is_file() {
                continue;
            }
            entries.push(FileEntry {
                directory: canonical.is_dir(),
                path: entry.path(),
            });
        }
        if entries.len() > 1000 {
            return Err(i18n::tr(
                "right.too_many",
                "目录超过 1000 项，请选择更小的目录",
            ));
        }
        file_cancelled(cancel)?;
        entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.path.cmp(&b.path)));
        file_cancelled(cancel)?;
        Ok(FileData::List(entries))
    }
}

fn comparison_lines(draft: &str, disk: &str) -> Vec<String> {
    let identical = draft == disk;
    let draft: Vec<_> = draft.lines().collect();
    let disk: Vec<_> = disk.lines().collect();
    let prefix = draft.iter().zip(&disk).take_while(|(a, b)| a == b).count();
    let suffix = draft[prefix..]
        .iter()
        .rev()
        .zip(disk[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut lines = vec!["--- 编辑草稿".into(), "+++ 磁盘内容".into()];
    if identical {
        lines.push("内容相同；文件身份或元数据可能已改变".into());
    } else {
        lines.push(format!(
            "@@ 草稿 {} 行 / 磁盘 {} 行 @@",
            draft.len(),
            disk.len()
        ));
        if draft == disk {
            lines.push("换行格式或末尾换行已变化".into());
        }
        lines.extend(
            draft[..prefix]
                .iter()
                .rev()
                .take(3)
                .rev()
                .map(|line| format!(" {line}")),
        );
        lines.extend(
            draft[prefix..draft.len() - suffix]
                .iter()
                .map(|line| format!("-{line}")),
        );
        lines.extend(
            disk[prefix..disk.len() - suffix]
                .iter()
                .map(|line| format!("+{line}")),
        );
        lines.extend(
            disk[disk.len() - suffix..]
                .iter()
                .take(3)
                .map(|line| format!(" {line}")),
        );
    }
    lines
}

fn draw_search_field(frame: &mut Frame<'_>, area: Rect, label: &str, value: &str, focused: bool) {
    use unicode_width::UnicodeWidthStr;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::current();
    let label_width = label.width().min(area.width as usize);
    let available = (area.width as usize).saturating_sub(label_width + 1);
    let display = value.replace(['\n', '\r', '\t'], " ");
    let mut start = display.len();
    for (byte, _) in display.char_indices().rev() {
        if display[byte..].width() > available {
            break;
        }
        start = byte;
    }
    let visible = &display[start..];
    frame.render_widget(
        Paragraph::new(format!("{label}{visible}")).style(theme.fg(theme.rose).bg(theme.hairline)),
        area,
    );
    if focused {
        let column = (label_width + visible.width()).min(area.width.saturating_sub(1) as usize);
        frame.set_cursor_position((area.x + column as u16, area.y));
    }
}

impl RightPanel {
    pub(super) fn draw_files(
        &self,
        frame: &mut Frame<'_>,
        pointer: Option<(u16, u16)>,
        mut body: Rect,
        hits: &mut Vec<(Rect, Action)>,
        editor_focus: bool,
    ) -> Option<Rect> {
        let theme = Theme::current();
        if body.height == 0 || body.width == 0 {
            return None;
        }
        let files = &self.files;
        self.explorer_area.set(None);
        if editor_focus && body.width >= 72 && files.preview_path.is_some() {
            let explorer = Rect::new(body.x, body.y, (body.width / 4).clamp(18, 28), body.height);
            self.explorer_area.set(Some(explorer));
            frame.render_widget(
                Block::default()
                    .borders(Borders::RIGHT)
                    .border_style(theme.fg(theme.hairline)),
                explorer,
            );
            frame.render_widget(
                Paragraph::new(if self.explorer_focused {
                    "● 文件 / F6编辑"
                } else {
                    "文件 / F6聚焦"
                })
                .style(theme.fg(theme.rose)),
                Rect::new(explorer.x, explorer.y, explorer.width.saturating_sub(1), 1),
            );
            let count = explorer.height.saturating_sub(1) as usize;
            let start = files.selected.saturating_sub(count.saturating_sub(1));
            for (row, (index, entry)) in files
                .entries
                .iter()
                .enumerate()
                .skip(start)
                .take(count)
                .enumerate()
            {
                let rect = Rect::new(
                    explorer.x,
                    explorer.y + row as u16 + 1,
                    explorer.width.saturating_sub(1),
                    1,
                );
                let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} {}{} {}",
                        if index == files.selected { "›" } else { " " },
                        "  ".repeat(
                            entry
                                .path
                                .strip_prefix(&files.directory)
                                .unwrap_or(&entry.path)
                                .components()
                                .count()
                                .saturating_sub(1)
                        ),
                        if entry.directory {
                            if files.expanded.contains(&entry.path) {
                                "▾"
                            } else {
                                "▸"
                            }
                        } else {
                            " "
                        },
                        name,
                    ))
                    .style(
                        if files.preview_path.as_ref() == Some(&entry.path) {
                            theme.fg(theme.rose).bg(theme.hairline)
                        } else {
                            theme.fg(theme.text)
                        },
                    ),
                    rect,
                );
                hits.insert(0, (rect, Action::File(index)));
            }
            body.x += explorer.width;
            body.width = body.width.saturating_sub(explorer.width);
        }
        let tabs_y = body.y;
        if !files.documents.is_empty() && body.height >= 4 {
            let arrows =
                files.documents.len() > (body.width / 12).max(1) as usize && body.width > 4;
            let tabs_width = body.width.saturating_sub(if arrows { 4 } else { 0 });
            if arrows {
                for (offset, label, backwards) in [(0, "‹", true), (2, "›", false)] {
                    let rect = Rect::new(body.right() - 4 + offset, tabs_y, 2, 1);
                    frame.render_widget(Paragraph::new(label).style(theme.fg(theme.rose)), rect);
                    hits.insert(0, (rect, Action::DocumentCycle(backwards)));
                }
            }
            let capacity = (tabs_width / 12).max(1) as usize;
            let count = files.documents.len().min(capacity);
            let start = files
                .active_document
                .unwrap_or(0)
                .saturating_sub(count.saturating_sub(1));
            let tab_width = (tabs_width / count as u16).max(1);
            for (visible, (index, document)) in files
                .documents
                .iter()
                .enumerate()
                .skip(start)
                .take(count)
                .enumerate()
            {
                let x = body.x + visible as u16 * tab_width;
                let width = if visible + 1 == count {
                    (body.x + tabs_width).saturating_sub(x)
                } else {
                    tab_width.min(body.right().saturating_sub(x))
                };
                let rect = Rect::new(x, tabs_y, width, 1);
                let name = document
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let dirty = document.editor.as_ref().is_some_and(Editor::dirty)
                    || files.active_document == Some(index) && files.dirty();
                let selected = files.active_document == Some(index);
                let close_rect = Rect::new(
                    rect.right().saturating_sub(1),
                    rect.y,
                    u16::from(width > 1),
                    1,
                );
                frame.render_widget(
                    Paragraph::new(layout::clip(
                        &format!(
                            "{}{}{}{}",
                            if visible == 0 && start > 0 {
                                "‹ "
                            } else {
                                ""
                            },
                            name,
                            if dirty { " •" } else { "" },
                            if visible + 1 == count && start + count < files.documents.len() {
                                " ›"
                            } else {
                                ""
                            }
                        ),
                        width.saturating_sub(2) as usize,
                    ))
                    .style(if selected {
                        theme.fg(theme.rose).bg(theme.hairline)
                    } else {
                        theme.fg(theme.text_dim)
                    }),
                    rect,
                );
                hits.insert(0, (rect, Action::Document(document.path.clone())));
                if width > 1 {
                    frame.render_widget(
                        Paragraph::new("×").style(theme.fg(theme.text_dim)),
                        close_rect,
                    );
                    hits.insert(
                        0,
                        (close_rect, Action::CloseDocument(document.path.clone())),
                    );
                }
            }
        }
        let top = if files.documents.is_empty() || body.height < 4 {
            body.y
        } else {
            body.y + 1
        };
        let save_width = body.width.min(9);
        let back = Rect::new(body.x, top, body.width.saturating_sub(save_width), 1);
        let path = files.preview_path.as_ref().unwrap_or(&files.directory);
        let relative = path.strip_prefix(&files.root).unwrap_or(path);
        let label = if relative.as_os_str().is_empty() {
            files.root.display().to_string()
        } else {
            format!("/{}", relative.display())
        };
        frame.render_widget(
            Paragraph::new(format!(
                "‹ {}{}",
                label,
                if files.dirty() { " *" } else { "" }
            ))
            .style(theme.fg(theme.rose)),
            back,
        );
        hits.insert(
            0,
            (
                back,
                if files.preview_path.is_some() {
                    Action::Explorer
                } else {
                    Action::Back
                },
            ),
        );
        let button = Rect::new(back.right(), top, save_width, 1);
        let editing = files.editor.is_some();
        frame.render_widget(
            Paragraph::new(if editing {
                i18n::tr("right.file_save", "[保存 ^S]")
            } else {
                i18n::tr("right.file_refresh", "[刷新 r]")
            })
            .style(theme.fg(theme.rose)),
            button,
        );
        hits.insert(
            0,
            (
                button,
                if editing {
                    Action::FileSave
                } else {
                    Action::Refresh
                },
            ),
        );
        let mut content = Rect::new(
            body.x,
            top + 1,
            body.width,
            body.bottom().saturating_sub(top + 2),
        );
        let search_bar = files
            .editor
            .as_ref()
            .filter(|editor| editor.search_active)
            .map(|editor| {
                let rows = if editor.replace_mode && !editor.goto_mode {
                    2
                } else {
                    1
                };
                let bar = Rect::new(
                    content.x,
                    content.y,
                    content.width,
                    rows.min(content.height),
                );
                content.y += bar.height;
                content.height = content.height.saturating_sub(bar.height);
                bar
            });
        let mut position;
        if let Some(preview) = files
            .editor
            .as_ref()
            .map(|e| e.text.as_str())
            .or(files.preview.as_deref())
        {
            let lines = highlight::lines(path, preview, theme.is_dark());
            let total = lines.len();
            let max = total.saturating_sub(content.height as usize);
            files.max_scroll.set(max);
            let offset = files.scroll.min(max);
            let digits = total.max(1).to_string().len();
            let gutter = (digits as u16 + 1).min(content.width);
            let width = content.width.saturating_sub(gutter);
            let max_width = lines.iter().map(|l| l.width()).max().unwrap_or(0);
            files
                .max_horizontal
                .set(max_width.saturating_sub(width as usize));
            // The cursor needs one extra display cell at the end of a line.
            let horizontal = files
                .horizontal
                .min(files.max_horizontal.get() + usize::from(editing));
            let matches: Vec<_> = files
                .editor
                .as_ref()
                .filter(|editor| {
                    editor.search_active && !editor.goto_mode && !editor.search.is_empty()
                })
                .map(|editor| {
                    preview
                        .match_indices(&editor.search)
                        .map(|(start, matched)| start..start + matched.len())
                        .collect()
                })
                .unwrap_or_default();
            for (row, (index, line)) in lines
                .into_iter()
                .enumerate()
                .skip(offset)
                .take(content.height as usize)
                .enumerate()
            {
                let number = Rect::new(
                    content.x,
                    content.y + row as u16,
                    gutter.saturating_sub(1),
                    1,
                );
                frame.render_widget(
                    Paragraph::new(format!("{:>digits$}", index + 1))
                        .style(theme.fg(theme.text_dim)),
                    number,
                );
                let text = Rect::new(content.x + gutter, number.y, width, 1);
                frame.render_widget(
                    Paragraph::new(line).scroll((0, horizontal.min(u16::MAX as usize) as u16)),
                    text,
                );
                let selection = files.editor.as_ref().and_then(Editor::selection);
                if selection.is_some() || !matches.is_empty() {
                    let start = preview
                        .split_inclusive('\n')
                        .take(index)
                        .map(str::len)
                        .sum::<usize>();
                    let raw = preview[start..].split('\n').next().unwrap_or("");
                    let mut column = 0usize;
                    for (byte, ch) in raw.char_indices() {
                        let cells = if ch == '\t' {
                            4
                        } else {
                            unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0)
                        };
                        let selected = selection
                            .as_ref()
                            .is_some_and(|selection| selection.contains(&(start + byte)));
                        let match_index =
                            matches.partition_point(|range| range.end <= start + byte);
                        let matched = matches
                            .get(match_index)
                            .is_some_and(|range| range.contains(&(start + byte)));
                        if selected || matched {
                            for cell in column..column + cells {
                                if cell >= horizontal && cell - horizontal < width as usize {
                                    frame.buffer_mut()
                                        [(text.x + (cell - horizontal) as u16, text.y)]
                                        .set_bg(if selected {
                                            theme.bg_select
                                        } else {
                                            theme.hairline
                                        });
                                    if matched && !selected {
                                        frame.buffer_mut()
                                            [(text.x + (cell - horizontal) as u16, text.y)]
                                            .set_style(
                                                Style::default().add_modifier(Modifier::UNDERLINED),
                                            );
                                    }
                                }
                            }
                        }
                        column += cells;
                    }
                }
            }
            position = format!(
                "{} · {}–{}/{}",
                highlight::language(path, preview),
                offset + 1,
                (offset + content.height as usize).min(total),
                total
            );
            if let Some(editor) = &files.editor {
                if let Some(bar) = search_bar {
                    let replacement_row =
                        editor.replace_mode && !editor.goto_mode && editor.replacement_focused;
                    for row in 0..bar.height {
                        let replacing = row == 1 || bar.height == 1 && replacement_row;
                        let active = replacing == replacement_row;
                        let label = if editor.goto_mode {
                            "行: "
                        } else if replacing {
                            "替: "
                        } else {
                            "找: "
                        };
                        let value = if replacing {
                            &editor.replacement
                        } else {
                            &editor.search
                        };
                        let field = Rect::new(bar.x, bar.y + row, bar.width, 1);
                        draw_search_field(
                            frame,
                            field,
                            label,
                            value,
                            self.focused && active && files.pending_leave.is_none(),
                        );
                    }
                }
                let (row, column) = editor.position();
                position = format!(
                    "{}{} {}:{} · {}",
                    highlight::language(path, preview),
                    if editor.dirty() { " *" } else { "" },
                    row + 1,
                    column + 1,
                    "^S保存 ^F查找 ^H替换 F6文件 F11专注"
                );
                if editor.search_active {
                    let (current, total) = editor.search_position();
                    position = if editor.goto_mode {
                        "Enter跳转 Esc关闭".into()
                    } else if editor.replace_mode {
                        format!("{current}/{total} Tab切换 Enter替换 ^Enter全部 Esc关闭")
                    } else {
                        format!("{current}/{total} Enter下一处 Shift+Enter上一处 Esc关闭")
                    };
                }
                let display_column = unicode_width::UnicodeWidthStr::width(
                    editor.text[..editor.cursor]
                        .rsplit('\n')
                        .next()
                        .unwrap_or("")
                        .replace('\t', "    ")
                        .as_str(),
                );
                if self.focused
                    && !self.explorer_focused
                    && !editor.search_active
                    && files.comparison.is_none()
                    && files.pending_leave.is_none()
                    && row >= offset
                    && row - offset < content.height as usize
                    && display_column >= horizontal
                    && display_column - horizontal < width as usize
                {
                    frame.set_cursor_position((
                        content.x + gutter + (display_column - horizontal) as u16,
                        content.y + (row - offset) as u16,
                    ));
                }
            }
            if !files.message.is_empty() {
                position = format!("{} · {position}", files.message);
            }
        } else if !files.message.is_empty() {
            frame.render_widget(
                Paragraph::new(files.message.as_str()).wrap(Wrap { trim: false }),
                content,
            );
            position = String::new();
        } else {
            let start = files
                .selected
                .saturating_sub(content.height.saturating_sub(1) as usize);
            for (row, (index, entry)) in files
                .entries
                .iter()
                .enumerate()
                .skip(start)
                .take(content.height as usize)
                .enumerate()
            {
                let rect = Rect::new(content.x, content.y + row as u16, content.width, 1);
                let label = format!(
                    "{} {}{}",
                    if index == files.selected { "›" } else { " " },
                    entry.path.file_name().unwrap_or_default().to_string_lossy(),
                    if entry.directory { "/" } else { "" }
                );
                let style = if index == files.selected || layout::hovered(rect, pointer) {
                    theme.fg(theme.rose).bg(theme.hairline)
                } else {
                    theme.fg(theme.text)
                };
                frame.render_widget(Paragraph::new(label).style(style), rect);
                hits.insert(0, (rect, Action::File(index)));
            }
            position = format!(
                "{} / {}",
                if files.entries.is_empty() {
                    0
                } else {
                    files.selected + 1
                },
                files.entries.len()
            );
        }
        if body.height > 1 {
            frame.render_widget(
                Paragraph::new(position).style(theme.fg(theme.text_dim)),
                Rect::new(body.x, body.bottom() - 1, body.width, 1),
            );
        }
        if let Some(comparison) = &files.comparison {
            hits.clear();
            frame.render_widget(ratatui::widgets::Clear, body);
            let header = Rect::new(body.x, body.y, body.width, body.height.min(2));
            frame.render_widget(
                Paragraph::new("磁盘差异 · Esc保留草稿\nR重新加载并丢弃草稿 · ↑↓滚动")
                    .style(theme.fg(theme.rose)),
                header,
            );
            if header.height > 0 {
                hits.push((
                    Rect::new(header.x, header.y, header.width, 1),
                    Action::CompareKeep,
                ));
            }
            if header.height > 1 {
                hits.push((
                    Rect::new(header.x, header.y + 1, header.width, 1),
                    Action::CompareReload,
                ));
            }
            let diff = Rect::new(
                body.x,
                header.bottom(),
                body.width,
                body.height.saturating_sub(header.height),
            );
            for (row, line) in comparison
                .lines
                .iter()
                .skip(comparison.scroll)
                .take(diff.height as usize)
                .enumerate()
            {
                let style = if line.starts_with('+') {
                    theme.fg(theme.rose).bg(theme.hairline)
                } else if line.starts_with('-') {
                    theme.fg(theme.text_dim).add_modifier(Modifier::CROSSED_OUT)
                } else {
                    theme.fg(theme.text)
                };
                frame.render_widget(
                    Paragraph::new(line.as_str()).style(style),
                    Rect::new(diff.x, diff.y + row as u16, diff.width, 1),
                );
            }
            return None;
        }
        if files.pending_leave.is_some() {
            frame.render_widget(ratatui::widgets::Clear, content);
            frame.render_widget(
                Paragraph::new(i18n::tr(
                    "right.file_unsaved",
                    "文件有未保存修改。请选择：\n保存后离开 / 放弃修改 / 取消离开",
                ))
                .wrap(Wrap { trim: false })
                .style(theme.fg(theme.rose)),
                content,
            );
            let labels = [
                (
                    i18n::tr("right.file_confirm_save", "[S 保存]"),
                    Action::FileConfirm(true),
                ),
                (
                    i18n::tr("right.file_confirm_discard", "[D 放弃]"),
                    Action::FileConfirm(false),
                ),
                (
                    i18n::tr("right.file_confirm_cancel", "[C 取消]"),
                    Action::FileCancel,
                ),
            ];
            for (index, (label, action)) in labels.into_iter().enumerate() {
                if index + 3 >= content.height as usize {
                    break;
                }
                let rect = Rect::new(content.x, content.y + index as u16 + 3, content.width, 1);
                frame.render_widget(Paragraph::new(label).style(theme.fg(theme.rose)), rect);
                hits.insert(0, (rect, action));
            }
            if !files.message.is_empty() && content.height > 7 {
                frame.render_widget(
                    Paragraph::new(files.message.as_str()).wrap(Wrap { trim: false }),
                    Rect::new(content.x, content.y + 7, content.width, content.height - 7),
                );
            }
        }
        files.editor.as_ref().map(|_| content)
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;
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
        assert_eq!(files.editor.as_ref().unwrap().text, "draft original\n");
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
        assert_eq!(files.editor.as_ref().unwrap().text, "newer\n");
        assert!(!files.dirty());
        assert_eq!(fs::read_to_string(&path).unwrap(), "newer\n");
        assert_eq!(files.documents[0].editor.as_ref().unwrap().text, "newer\n");
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
        editor.search_active = true;
        editor.replace_mode = true;
        editor.search = "match".into();
        editor.replacement = "很长的中文替换内容🙂".repeat(5);
        editor.replacement_focused = true;
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
                assert!(
                    buffer[(code.x + 7, code.y)]
                        .modifier
                        .contains(Modifier::UNDERLINED)
                );
                assert!(
                    buffer[(code.x + 13, code.y)]
                        .modifier
                        .contains(Modifier::UNDERLINED)
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
        assert_eq!(files.editor.as_ref().unwrap().text, "drafthello");
        files
            .editor
            .as_mut()
            .unwrap()
            .key(crossterm::event::KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL,
            ));
        assert_eq!(files.editor.as_ref().unwrap().text, "hello");
    }
    #[test]
    fn only_complete_utf8_text_is_editable() {
        for bytes in [vec![b'a'; 65537], vec![0, 1], vec![0xff, b'a']] {
            let (_root, files) = opened(&bytes);
            assert!(files.editor.is_none());
            assert!(files.message.contains("只读"));
        }
        let (_root, files) = opened(b"");
        assert_eq!(files.editor.as_ref().unwrap().text, "");
    }
    #[test]
    fn loading_another_file_preserves_active_draft_and_undo() {
        let (root, mut files) = opened(b"hello");
        let path = root.path().join("example.rs");
        files.register_active_document(path.clone());
        files.editor.as_mut().unwrap().insert("draft");
        files.load(root.path().join("other.rs"), true);
        assert!(files.activate_path(&path));
        assert_eq!(files.editor.as_ref().unwrap().text, "drafthello");
        files
            .editor
            .as_mut()
            .unwrap()
            .key(crossterm::event::KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL,
            ));
        assert_eq!(files.editor.as_ref().unwrap().text, "hello");
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
        assert_eq!(files.editor.as_ref().unwrap().text, "drafthello");
        files.pending_leave = Some(Leave::CloseDocument(path));
        assert!(files.confirm(false).is_some());
        assert!(files.documents.is_empty());
        assert!(files.active_document.is_none());
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
