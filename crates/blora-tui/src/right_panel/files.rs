// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use super::editor::{Editor, Preview};
use super::*;

pub(super) const MAX_EDITABLE_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const MAX_PREVIEW_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) enum Leave {
    Back,
    Refresh,
    Close,
    CloseBuffer(usize),
    Project(PathBuf),
}

#[derive(Debug)]
pub(super) struct FileEntry {
    pub(super) path: PathBuf,
    pub(super) directory: bool,
}

pub(super) struct FileResult {
    pub(super) generation: u64,
    pub(super) path: PathBuf,
    pub(super) result: Result<FileData, String>,
}

pub(super) enum FileData {
    List(Vec<FileEntry>),
    Document {
        preview: Option<String>,
        editor: Option<Box<Editor>>,
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
                    path: request.path,
                    result,
                });
            }
        }
    }
}

pub(super) struct Files {
    pub(super) buffers: Vec<Editor>,
    pub(super) active: Option<usize>,
    pub(super) preview: Option<Preview>,
    pub(super) preview_path: Option<PathBuf>,
    pub(super) pending_leave: Option<Leave>,
    pub(super) blocked_project: Option<PathBuf>,
    pub(super) root: PathBuf,
    pub(super) directory: PathBuf,
    pub(super) generation: u64,
    pub(super) entries: Vec<FileEntry>,
    pub(super) selected: usize,
    pub(super) horizontal: usize,
    pub(super) max_scroll: std::cell::Cell<usize>,
    pub(super) max_horizontal: std::cell::Cell<usize>,
    pub(super) gutter_width: std::cell::Cell<usize>,
    pub(super) viewport_rows: std::cell::Cell<usize>,
    positions: std::collections::HashMap<PathBuf, (usize, usize)>,
    restore: Option<PathBuf>,
    pub(super) message: String,
    pub(super) worker: std::sync::Arc<FileWorker>,
    pub(super) handle: Option<thread::JoinHandle<()>>,
    pub(super) cancel: CancelToken,
    pub(super) scroll: usize,
    pub(super) click: Option<(std::time::Instant, u16, u16)>,
    pub(super) click_count: u8,
    pub(super) drag_anchor: Option<usize>,
    pub(super) dragging: bool,
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
    pub(super) fn new() -> Self {
        let worker = std::sync::Arc::new(FileWorker::default());
        let background = worker.clone();
        let handle = thread::Builder::new()
            .name("blora-right-files".into())
            .spawn(move || background.run())
            .ok();
        Self {
            buffers: vec![],
            active: None,
            preview: None,
            preview_path: None,
            pending_leave: None,
            blocked_project: None,
            root: PathBuf::new(),
            directory: PathBuf::new(),
            generation: 0,
            entries: vec![],
            selected: 0,
            horizontal: 0,
            max_scroll: 0.into(),
            max_horizontal: 0.into(),
            gutter_width: 0.into(),
            viewport_rows: 10.into(),
            positions: Default::default(),
            restore: None,
            message: String::new(),
            worker,
            handle,
            cancel: CancelToken::new(),
            scroll: 0,
            click: None,
            click_count: 0,
            drag_anchor: None,
            dragging: false,
        }
    }

    pub(super) fn editor(&self) -> Option<&Editor> {
        self.active.and_then(|index| self.buffers.get(index))
    }

    pub(super) fn editor_mut(&mut self) -> Option<&mut Editor> {
        self.active
            .and_then(move |index| self.buffers.get_mut(index))
    }

    pub(super) fn showing_editor(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn showing_preview(&self) -> bool {
        self.preview.is_some()
    }

    fn save_position(&mut self) {
        if let Some(editor) = self.editor() {
            self.positions
                .insert(editor.path().to_path_buf(), (self.scroll, self.horizontal));
        }
    }

    fn restore_position(&mut self, path: &Path) {
        let (scroll, horizontal) = self.positions.get(path).copied().unwrap_or((0, 0));
        self.scroll = scroll;
        self.horizontal = horizontal;
    }

    fn activate_existing(&mut self, path: &Path) -> bool {
        let Some(index) = self.buffers.iter().position(|editor| editor.path() == path) else {
            return false;
        };
        self.save_position();
        self.active = Some(index);
        self.preview = None;
        self.preview_path = None;
        self.message.clear();
        self.restore_position(path);
        true
    }

    pub(super) fn load(&mut self, path: PathBuf, preview: bool) {
        if preview {
            self.cancel();
            if self.activate_existing(&path) {
                return;
            }
            self.message = i18n::tr("right.loading", "加载中…");
            self.preview_path = Some(path.clone());
            self.send(path, true);
        } else {
            self.save_position();
            self.active = None;
            self.preview = None;
            self.preview_path = None;
            self.scroll = 0;
            self.horizontal = 0;
            self.directory = path.clone();
            self.entries.clear();
            self.selected = 0;
            self.message = i18n::tr("right.loading", "加载中…");
            self.send(path, false);
        }
    }

    fn send(&mut self, path: PathBuf, preview: bool) {
        self.cancel();
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
        self.cancel.cancel();
        self.generation = self.generation.wrapping_add(1);
        let mut work = self.worker.work.lock().unwrap();
        work.pending = None;
        work.result = None;
    }

    pub(super) fn close(&mut self) {
        self.buffers.clear();
        self.active = None;
        self.preview = None;
        self.preview_path = None;
        self.pending_leave = None;
        self.blocked_project = None;
        self.cancel();
        self.root.clear();
        self.directory.clear();
        self.entries.clear();
        self.positions.clear();
        self.restore = None;
        self.message.clear();
    }

    pub(super) fn sync(&mut self, root: &Path) {
        if self.root != root {
            if self.any_dirty() {
                if self.blocked_project.as_deref() != Some(root) && self.pending_leave.is_none() {
                    self.pending_leave = Some(Leave::Project(root.to_path_buf()));
                }
                return;
            }
            self.blocked_project = None;
            self.positions.clear();
            self.restore = None;
            self.buffers.clear();
            self.active = None;
            self.preview = None;
            self.preview_path = None;
            self.directory.clear();
            self.root = root.to_path_buf();
            self.load(root.to_path_buf(), false);
        }
        let update = self.worker.work.lock().unwrap().result.take();
        if let Some(update) = update {
            if update.generation != self.generation {
                return;
            }
            let path = update.path;
            match update.result {
                Ok(FileData::List(entries)) => {
                    self.entries = entries;
                    if let Some(restore) = self.restore.take()
                        && let Some(index) =
                            self.entries.iter().position(|entry| entry.path == restore)
                    {
                        self.selected = index;
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
                    self.message = reason;
                    if let Some(editor) = editor {
                        let existing = self.buffers.iter().position(|buffer| buffer.path() == path);
                        let index = match existing {
                            Some(index) => {
                                self.buffers[index] = *editor;
                                index
                            }
                            None => {
                                self.buffers.push(*editor);
                                self.buffers.len() - 1
                            }
                        };
                        self.active = Some(index);
                        self.preview = None;
                        self.preview_path = None;
                        self.restore_position(&path);
                    } else {
                        if let Some(index) =
                            self.buffers.iter().position(|buffer| buffer.path() == path)
                        {
                            self.buffers.remove(index);
                        }
                        self.active = None;
                        self.preview_path = Some(path.clone());
                        self.preview = preview
                            .map(|text| Preview::new(&path, text, Theme::current().is_dark()));
                    }
                }
                Err(error) => {
                    if let Some(index) =
                        self.buffers.iter().position(|buffer| buffer.path() == path)
                    {
                        self.remove_buffer(index);
                    }
                    self.active = None;
                    self.preview = None;
                    self.preview_path = None;
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
            self.load(path, preview);
        }
    }

    pub(super) fn dirty(&self) -> bool {
        self.editor().is_some_and(Editor::dirty)
    }

    pub(super) fn any_dirty(&self) -> bool {
        self.buffers.iter().any(Editor::dirty)
    }

    pub(super) fn save(&mut self) -> bool {
        let root = self.root.clone();
        if let Some(editor) = self.editor_mut() {
            match editor.save(&root) {
                Ok(()) => {
                    self.message = i18n::tr("right.file_saved", "已保存");
                    return true;
                }
                Err(error) => self.message = error,
            }
        }
        false
    }

    fn save_all(&mut self) -> bool {
        let root = self.root.clone();
        for editor in &mut self.buffers {
            if !editor.dirty() {
                continue;
            }
            if let Err(error) = editor.save(&root) {
                self.message = error;
                return false;
            }
        }
        self.message = i18n::tr("right.file_saved", "已保存");
        true
    }

    pub(super) fn confirm(&mut self, save: bool) -> Option<Leave> {
        if save && !self.save_all() {
            return None;
        }
        let leave = self.pending_leave.take()?;
        if !save {
            match &leave {
                Leave::Back => {
                    if let Some(index) = self.active {
                        self.remove_buffer(index);
                    }
                }
                Leave::Refresh => {
                    if let Some(editor) = self.editor_mut() {
                        editor.discard_changes();
                    }
                }
                Leave::Project(_) => {
                    self.buffers.clear();
                    self.active = None;
                    self.positions.clear();
                }
                Leave::Close | Leave::CloseBuffer(_) => {}
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
        if let Some(editor) = self.editor() {
            let path = editor.path().to_path_buf();
            self.save_position();
            self.message = i18n::tr("right.loading", "加载中…");
            self.send(path, true);
            return;
        }
        if let Some(path) = self.preview_path.clone() {
            self.message = i18n::tr("right.loading", "加载中…");
            self.send(path, true);
            return;
        }
        let path = self.directory.clone();
        self.restore = self
            .entries
            .get(self.selected)
            .map(|entry| entry.path.clone());
        self.load(path, false);
    }

    pub(super) fn back(&mut self) {
        if self.dirty() {
            self.pending_leave = Some(Leave::Back);
            return;
        }
        self.save_position();
        self.message.clear();
        if self.preview.take().is_some() {
            self.preview_path = None;
            self.scroll = 0;
            self.horizontal = 0;
            if self.entries.is_empty() {
                self.load(self.directory.clone(), false);
            }
        } else if self.active.take().is_some() {
            self.scroll = 0;
            self.horizontal = 0;
            if self.entries.is_empty() && !self.directory.as_os_str().is_empty() {
                self.load(self.directory.clone(), false);
            }
        } else if self.directory != self.root {
            let parent = self.directory.parent().map(Path::to_path_buf);
            if let Some(parent) = parent {
                self.restore = Some(self.directory.clone());
                self.load(parent, false);
            }
        }
    }

    pub(super) fn switch_to(&mut self, index: usize) {
        if index >= self.buffers.len() {
            return;
        }
        self.save_position();
        self.active = Some(index);
        self.preview = None;
        self.preview_path = None;
        let path = self.buffers[index].path().to_path_buf();
        self.restore_position(&path);
        self.message.clear();
    }

    pub(super) fn switch_buffer(&mut self, forward: bool) -> bool {
        if self.buffers.len() < 2 {
            return false;
        }
        self.save_position();
        let current = self.active.unwrap_or(0);
        let len = self.buffers.len();
        let next = if forward {
            (current + 1) % len
        } else if current == 0 {
            len - 1
        } else {
            current - 1
        };
        self.active = Some(next);
        self.preview = None;
        self.preview_path = None;
        let path = self.buffers[next].path().to_path_buf();
        self.restore_position(&path);
        self.message.clear();
        true
    }

    pub(super) fn close_buffer(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        if self.buffers[index].dirty() {
            self.pending_leave = Some(Leave::CloseBuffer(index));
            return;
        }
        self.remove_buffer(index);
    }

    pub(super) fn remove_buffer(&mut self, index: usize) {
        if index >= self.buffers.len() {
            return;
        }
        let path = self.buffers[index].path().to_path_buf();
        self.positions.remove(&path);
        self.buffers.remove(index);
        if self.buffers.is_empty() {
            self.active = None;
            self.scroll = 0;
            self.horizontal = 0;
        } else {
            let next = index.min(self.buffers.len() - 1);
            self.active = Some(next);
            let path = self.buffers[next].path().to_path_buf();
            self.restore_position(&path);
        }
    }
}

#[cfg(test)]
pub(super) fn read_file_data(root: &Path, path: &Path, preview: bool) -> Result<FileData, String> {
    read_file_data_cancelled(root, path, preview, &CancelToken::new())
}

pub(super) fn file_cancelled(cancel: &CancelToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("file request cancelled".to_owned())
    } else {
        Ok(())
    }
}

pub(super) fn read_file_data_cancelled(
    root: &Path,
    path: &Path,
    preview: bool,
    cancel: &CancelToken,
) -> Result<FileData, String> {
    file_cancelled(cancel)?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    file_cancelled(cancel)?;
    let requested_path = path;
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    file_cancelled(cancel)?;
    if !path.starts_with(&root) {
        return Err(i18n::tr("right.boundary", "路径超出项目边界"));
    }
    if !preview {
        let mut entries = Vec::new();
        file_cancelled(cancel)?;
        let mut directory = fs::read_dir(&path).map_err(|error| error.to_string())?;
        for _ in 0..1001 {
            file_cancelled(cancel)?;
            let Some(entry) = directory.next() else {
                break;
            };
            file_cancelled(cancel)?;
            let entry = entry.map_err(|error| error.to_string())?;
            let canonical = match entry.path().canonicalize() {
                Ok(path) if path.starts_with(&root) => path,
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
        return Ok(FileData::List(entries));
    }
    if !path.is_file() {
        return Err(i18n::tr("right.special", "仅支持预览普通文件"));
    }
    file_cancelled(cancel)?;
    let mut file = fs::File::open(&path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    let limit = if metadata.len() <= MAX_EDITABLE_BYTES {
        MAX_EDITABLE_BYTES
    } else {
        MAX_PREVIEW_BYTES.min(MAX_EDITABLE_BYTES)
    };
    let truncated = metadata.len() > limit;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    while (bytes.len() as u64) < limit {
        file_cancelled(cancel)?;
        let remaining = (limit - bytes.len() as u64).min(chunk.len() as u64) as usize;
        let count = file
            .read(&mut chunk[..remaining])
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    file_cancelled(cancel)?;
    let valid = std::str::from_utf8(&bytes).is_ok();
    let binary = bytes
        .iter()
        .any(|byte| *byte == 0 || (*byte < 32 && !matches!(*byte, 9 | 10 | 13)));
    let editable = !binary && valid && !truncated && bytes.len() as u64 <= MAX_EDITABLE_BYTES;
    let reason = if editable {
        String::new()
    } else if truncated {
        i18n::tr("right.file_truncated", "只读 · 文件过大，仅显示前 1 MiB")
    } else if binary {
        i18n::tr("right.file_binary", "只读 · 二进制文件")
    } else if !valid {
        i18n::tr("right.file_invalid_utf8", "只读 · 无效 UTF-8")
    } else {
        String::new()
    };
    if editable {
        let editor = Editor::new(requested_path, bytes, metadata)?;
        return Ok(FileData::Document {
            preview: None,
            editor: Some(Box::new(editor)),
            reason,
        });
    }
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if binary {
        text = text
            .chars()
            .map(|ch| {
                if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
                    '�'
                } else {
                    ch
                }
            })
            .collect();
    }
    if truncated {
        text.push_str(&format!(
            "\n{}",
            i18n::tr("right.file_truncated_note", "仅显示文件前 1 MiB")
        ));
    }
    if text.is_empty() {
        text = i18n::tr("right.empty_file", "空文件");
    }
    Ok(FileData::Document {
        preview: Some(text),
        editor: None,
        reason,
    })
}

impl RightPanel {
    pub(super) fn draw_files(
        &self,
        frame: &mut Frame<'_>,
        model: &FrameModel<'_>,
        body: Rect,
        hits: &mut Vec<(Rect, Action)>,
    ) -> Option<Rect> {
        let theme = Theme::current();
        if body.height == 0 || body.width == 0 {
            return None;
        }
        let files = &self.files;
        let save_width = body.width.min(9);
        let back = Rect::new(body.x, body.y, body.width.saturating_sub(save_width), 1);
        let path = files
            .editor()
            .map(Editor::path)
            .or(files.preview_path.as_deref())
            .unwrap_or(&files.directory);
        let relative = path.strip_prefix(&files.root).unwrap_or(path);
        let edited = files.showing_editor();
        frame.render_widget(
            Paragraph::new(format!(
                "‹ /{}{}",
                relative.display(),
                if files.dirty() { " *" } else { "" }
            ))
            .style(theme.fg(theme.rose)),
            back,
        );
        hits.insert(0, (back, Action::Back));
        let button = Rect::new(back.right(), body.y, save_width, 1);
        frame.render_widget(
            Paragraph::new(if edited {
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
                if edited {
                    Action::FileSave
                } else {
                    Action::Refresh
                },
            ),
        );
        let mut top = body.y + 1;
        if files.buffers.len() > 1 {
            self.draw_buffer_tabs(frame, body, top, hits, &theme);
            top += 1;
        }
        if let Some(editor) = files.editor() {
            if let Some(goto) = editor.goto_view() {
                self.draw_goto_bar(frame, body, top, goto, &theme);
                top += 1;
            } else if let Some(find) = editor.find_view() {
                self.draw_find_bars(frame, body, top, &find, &theme);
                top += if find.replace { 2 } else { 1 };
            }
        }
        let status_row = body.bottom().saturating_sub(1);
        let content = Rect::new(body.x, top, body.width, status_row.saturating_sub(top));
        let mut position;
        if let Some(editor) = files.editor() {
            position = self.draw_editor(frame, editor, content, &theme);
        } else if let Some(preview) = files.preview.as_ref() {
            position = self.draw_preview(frame, preview, content, &theme);
        } else if !files.message.is_empty() && files.entries.is_empty() {
            frame.render_widget(
                Paragraph::new(files.message.as_str()).wrap(Wrap { trim: false }),
                content,
            );
            position = String::new();
        } else {
            position = self.draw_list(frame, model, content, hits, &theme);
        }
        if !files.message.is_empty()
            && (files.showing_editor() || files.showing_preview())
            && files.pending_leave.is_none()
        {
            position = format!("{} · {position}", files.message);
        }
        if body.height > 1 {
            frame.render_widget(
                Paragraph::new(position).style(theme.fg(theme.text_dim)),
                Rect::new(body.x, body.bottom() - 1, body.width, 1),
            );
        }
        if files.pending_leave.is_some() {
            frame.render_widget(ratatui::widgets::Clear, content);
            self.draw_leave_confirm(frame, content, hits, &theme);
        }
        files
            .showing_editor()
            .then_some(content)
            .or_else(|| files.showing_preview().then_some(content))
    }

    fn draw_buffer_tabs(
        &self,
        frame: &mut Frame<'_>,
        body: Rect,
        top: u16,
        hits: &mut Vec<(Rect, Action)>,
        theme: &Theme,
    ) {
        let count = self.files.buffers.len();
        let width = body.width / count as u16;
        for (index, editor) in self.files.buffers.iter().enumerate() {
            let x = body.x + width * index as u16;
            let rect = Rect::new(
                x,
                top,
                if index + 1 == count {
                    body.right().saturating_sub(x)
                } else {
                    width
                },
                1,
            );
            let name = editor
                .path()
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();
            let label = format!(
                "{}{}",
                if editor.dirty() { "●" } else { "" },
                layout::clip(&name, rect.width.saturating_sub(1) as usize)
            );
            let active = self.files.active == Some(index);
            let style = if active {
                theme
                    .fg(theme.rose)
                    .bg(theme.hairline)
                    .add_modifier(ratatui::style::Modifier::UNDERLINED)
            } else {
                theme.fg(theme.text_dim)
            };
            frame.render_widget(Paragraph::new(label).style(style), rect);
            hits.insert(0, (rect, Action::Buffer(index)));
        }
    }

    fn draw_goto_bar(
        &self,
        frame: &mut Frame<'_>,
        body: Rect,
        top: u16,
        text: &str,
        theme: &Theme,
    ) {
        let label = format!("{} {text}", i18n::tr("right.goto_prompt", "跳转行:"));
        frame.render_widget(
            Paragraph::new(label).style(theme.fg(theme.text).bg(theme.hairline)),
            Rect::new(body.x, top, body.width, 1),
        );
    }

    fn draw_find_bars(
        &self,
        frame: &mut Frame<'_>,
        body: Rect,
        top: u16,
        find: &super::editor::FindView<'_>,
        theme: &Theme,
    ) {
        let counter = if find.matches == 0 {
            "0/0".to_owned()
        } else {
            format!(
                "{}/{}",
                find.current.map_or(0, |index| index + 1),
                find.matches
            )
        };
        let case = if find.case_sensitive { "Aa" } else { "aa" };
        let mut spans = vec![ratatui::text::Span::styled(
            format!("{} ", i18n::tr("right.find_prompt", "查找:")),
            theme.fg(theme.text_dim),
        )];
        let query_style = if find.focus_replace {
            theme.fg(theme.text)
        } else {
            theme.fg(theme.text).bg(theme.hairline)
        };
        spans.push(ratatui::text::Span::styled(
            find.query.to_owned(),
            query_style,
        ));
        spans.push(ratatui::text::Span::styled(
            format!("  {counter} {case}"),
            theme.fg(theme.text_mute),
        ));
        if let Some(error) = find.error {
            spans.push(ratatui::text::Span::styled(
                format!("  {error}"),
                theme.fg(theme.rose),
            ));
        }
        frame.render_widget(
            Paragraph::new(ratatui::text::Line::from(spans)),
            Rect::new(body.x, top, body.width, 1),
        );
        if find.replace {
            let style = if find.focus_replace {
                theme.fg(theme.text).bg(theme.hairline)
            } else {
                theme.fg(theme.text)
            };
            let line = ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(
                    format!("{} ", i18n::tr("right.replace_prompt", "替换:")),
                    theme.fg(theme.text_dim),
                ),
                ratatui::text::Span::styled(find.replacement.to_owned(), style),
            ]);
            frame.render_widget(
                Paragraph::new(line),
                Rect::new(body.x, top + 1, body.width, 1),
            );
        }
    }

    fn draw_editor(
        &self,
        frame: &mut Frame<'_>,
        editor: &Editor,
        content: Rect,
        theme: &Theme,
    ) -> String {
        if content.height == 0 || content.width == 0 {
            return String::new();
        }
        let dark = theme.is_dark();
        let height = content.height as usize;
        let total = editor.line_count();
        let max = total.saturating_sub(height);
        self.files.max_scroll.set(max);
        let offset = self.files.scroll.min(max);
        let digits = total.max(1).to_string().len();
        let scrollbar = u16::from(total > height && content.width > 10);
        let gutter = (digits as u16 + 1).min(content.width.saturating_sub(scrollbar));
        let text_width = content
            .width
            .saturating_sub(gutter)
            .saturating_sub(scrollbar);
        self.files.gutter_width.set(gutter as usize);
        self.files.viewport_rows.set(height.max(1));
        editor.set_viewport_rows(height.max(1));
        editor.ensure_viewport(offset, height.max(1), dark);
        let max_width = editor.max_width(dark);
        self.files
            .max_horizontal
            .set(max_width.saturating_sub(text_width as usize));
        let horizontal = self
            .files
            .horizontal
            .min(self.files.max_horizontal.get() + 1);
        for row in 0..height {
            let index = offset + row;
            if index >= total {
                break;
            }
            let number = Rect::new(
                content.x,
                content.y + row as u16,
                gutter.saturating_sub(1),
                1,
            );
            frame.render_widget(
                Paragraph::new(format!("{:>digits$}", index + 1)).style(theme.fg(theme.text_dim)),
                number,
            );
            let rect = Rect::new(content.x + gutter, content.y + row as u16, text_width, 1);
            let line = editor.render_line(index, text_width as usize, horizontal, dark, theme);
            frame.render_widget(Paragraph::new(line), rect);
        }
        if scrollbar > 0 {
            self.draw_scrollbar(frame, content, offset, total, height, theme);
        }
        let (row, column) = editor.position();
        let display = editor.display_column(editor.cursor());
        if row >= offset
            && row - offset < height
            && display >= horizontal
            && display - horizontal < text_width as usize
        {
            frame.set_cursor_position((
                content.x + gutter + (display - horizontal) as u16,
                content.y + (row - offset) as u16,
            ));
        }
        let selection = editor
            .selection()
            .map(|(start, end)| editor.text()[start..end].chars().count());
        let mut status = format!(
            "{}  {}:{}  UTF-8  {}  {}/{}",
            editor.language(),
            row + 1,
            column + 1,
            editor.eol(),
            offset + 1,
            total
        );
        if let Some(count) = selection {
            status.push_str(&format!("  Sel {count}"));
        }
        status
    }

    fn draw_preview(
        &self,
        frame: &mut Frame<'_>,
        preview: &Preview,
        content: Rect,
        theme: &Theme,
    ) -> String {
        if content.height == 0 || content.width == 0 {
            return String::new();
        }
        let dark = theme.is_dark();
        let height = content.height as usize;
        let total = preview.line_count();
        let max = total.saturating_sub(height);
        self.files.max_scroll.set(max);
        let offset = self.files.scroll.min(max);
        let digits = total.max(1).to_string().len();
        let scrollbar = u16::from(total > height && content.width > 10);
        let gutter = (digits as u16 + 1).min(content.width.saturating_sub(scrollbar));
        let text_width = content
            .width
            .saturating_sub(gutter)
            .saturating_sub(scrollbar);
        self.files.gutter_width.set(gutter as usize);
        self.files.viewport_rows.set(height.max(1));
        preview.ensure_viewport(offset, height.max(1), dark);
        let max_width = preview.max_width(dark);
        self.files
            .max_horizontal
            .set(max_width.saturating_sub(text_width as usize));
        let horizontal = self.files.horizontal.min(self.files.max_horizontal.get());
        for row in 0..height {
            let index = offset + row;
            if index >= total {
                break;
            }
            frame.render_widget(
                Paragraph::new(format!("{:>digits$}", index + 1)).style(theme.fg(theme.text_dim)),
                Rect::new(
                    content.x,
                    content.y + row as u16,
                    gutter.saturating_sub(1),
                    1,
                ),
            );
            let line = preview.render_line(index, text_width as usize, horizontal, theme);
            frame.render_widget(
                Paragraph::new(line),
                Rect::new(content.x + gutter, content.y + row as u16, text_width, 1),
            );
        }
        if scrollbar > 0 {
            self.draw_scrollbar(frame, content, offset, total, height, theme);
        }
        format!("{}  {}/{}", preview.language(), offset + 1, total)
    }

    fn draw_scrollbar(
        &self,
        frame: &mut Frame<'_>,
        content: Rect,
        offset: usize,
        total: usize,
        height: usize,
        theme: &Theme,
    ) {
        let x = content.right().saturating_sub(1);
        let thumb = (height * height / total.max(1)).max(1).min(height);
        let start = if total <= height {
            0
        } else {
            offset * (height.saturating_sub(thumb)) / (total - height)
        };
        for row in 0..height {
            let ch = if row >= start && row < start + thumb {
                "█"
            } else {
                "│"
            };
            frame.render_widget(
                Paragraph::new(ch).style(theme.fg(theme.text_mute)),
                Rect::new(x, content.y + row as u16, 1, 1),
            );
        }
    }

    fn draw_list(
        &self,
        frame: &mut Frame<'_>,
        model: &FrameModel<'_>,
        content: Rect,
        hits: &mut Vec<(Rect, Action)>,
        theme: &Theme,
    ) -> String {
        let files = &self.files;
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
            let style = if index == files.selected || layout::hovered(rect, model.pointer) {
                theme.fg(theme.rose).bg(theme.hairline)
            } else {
                theme.fg(theme.text)
            };
            frame.render_widget(Paragraph::new(label).style(style), rect);
            hits.insert(0, (rect, Action::File(index)));
        }
        format!(
            "{} / {}",
            if files.entries.is_empty() {
                0
            } else {
                files.selected + 1
            },
            files.entries.len()
        )
    }

    fn draw_leave_confirm(
        &self,
        frame: &mut Frame<'_>,
        content: Rect,
        hits: &mut Vec<(Rect, Action)>,
        theme: &Theme,
    ) {
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
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;

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
        if let Some(editor) = editor {
            files.buffers.push(*editor);
            files.active = Some(0);
        } else {
            files.preview_path = Some(path);
            files.preview = preview.map(|text| Preview::new(Path::new("example.rs"), text, true));
        }
        files.message = reason;
        (root, files)
    }

    #[test]
    fn large_and_binary_files_are_read_only() {
        let (_root, files) = opened(&vec![b'a'; 65537]);
        assert_eq!(files.buffers.len(), 1);
        assert!(files.editor().is_some());
        for bytes in [
            vec![b'a'; MAX_EDITABLE_BYTES as usize + 1],
            vec![0, 1],
            vec![0xff, b'a'],
        ] {
            let (_root, files) = opened(&bytes);
            assert!(files.editor().is_none());
            assert!(files.message.contains("只读"));
        }
    }

    #[test]
    fn back_cancel_and_project_change_preserve_draft() {
        let (root, mut files) = opened(b"hello");
        files.editor_mut().unwrap().insert("draft");
        files.back();
        assert!(files.pending_leave.is_some());
        assert!(files.any_dirty());
        files.cancel_leave();
        assert!(files.pending_leave.is_none());
        assert!(files.any_dirty());
        let next = tempfile::tempdir().unwrap();
        files.sync(next.path());
        assert_eq!(files.root, root.path());
        assert!(matches!(files.pending_leave, Some(Leave::Project(_))));
        files.cancel_leave();
        files.sync(next.path());
        assert!(files.pending_leave.is_none());
        assert!(files.any_dirty());
    }

    #[test]
    fn failed_save_keeps_confirmation_and_draft() {
        let (root, mut files) = opened(b"hello");
        files.editor_mut().unwrap().insert("draft");
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

    #[test]
    fn buffers_switch_and_close_without_losing_edits() {
        let (root, mut files) = opened(b"hello");
        let path = root.path().join("second.rs");
        fs::write(&path, "second").unwrap();
        files.open(0);
        let metadata = fs::metadata(&path).unwrap();
        files
            .buffers
            .push(Editor::new(&path, b"second".to_vec(), metadata).unwrap());
        files.active = Some(1);
        assert_eq!(files.buffers.len(), 2);
        assert!(files.switch_buffer(false));
        assert_eq!(files.active, Some(0));
        files.close_buffer();
        assert_eq!(files.buffers.len(), 1);
    }
}
