// SPDX-License-Identifier: GPL-3.0-or-later
use super::editor::Editor;
use super::*;

#[derive(Clone, Debug)]
pub(super) enum Leave {
    Back,
    Refresh,
    Close,
    Project(PathBuf),
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
    pub(super) editor: Option<Editor>,
    pub(super) pending_leave: Option<Leave>,
    pub(super) blocked_project: Option<PathBuf>,
    pub(super) root: PathBuf,
    pub(super) directory: PathBuf,
    pub(super) generation: u64,
    pub(super) entries: Vec<FileEntry>,
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
    pub(super) fn new() -> Self {
        let worker = std::sync::Arc::new(FileWorker::default());
        let background = worker.clone();
        let handle = thread::Builder::new()
            .name("blora-right-files".into())
            .spawn(move || background.run())
            .ok();
        Self {
            editor: None,
            pending_leave: None,
            blocked_project: None,
            root: PathBuf::new(),
            directory: PathBuf::new(),
            generation: 0,
            entries: vec![],
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
        self.cancel.cancel();
        self.generation = self.generation.wrapping_add(1);
        let mut work = self.worker.work.lock().unwrap();
        work.pending = None;
        work.result = None;
    }
    pub(super) fn close(&mut self) {
        self.editor = None;
        self.pending_leave = None;
        self.blocked_project = None;
        self.cancel();
        self.root.clear();
        self.directory.clear();
        self.entries.clear();
        self.preview = None;
        self.preview_path = None;
        self.positions.clear();
        self.restore = None;
        self.message.clear();
    }

    pub(super) fn sync(&mut self, root: &Path) {
        if self.root != root {
            if self.dirty() {
                if self.blocked_project.as_deref() != Some(root) && self.pending_leave.is_none() {
                    self.pending_leave = Some(Leave::Project(root.to_path_buf()));
                }
                return;
            }
            self.blocked_project = None;
            self.positions.clear();
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
            self.load(path, preview);
        }
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
                Err(error) => self.message = error,
            }
        }
        false
    }
    pub(super) fn confirm(&mut self, save: bool) -> Option<Leave> {
        if save && !self.save() {
            return None;
        }
        let leave = self.pending_leave.take()?;
        self.editor = None;
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
pub(super) fn read_file_data_cancelled(
    root: &Path,
    path: &Path,
    preview: bool,
    cancel: &CancelToken,
) -> Result<FileData, String> {
    file_cancelled(cancel)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    file_cancelled(cancel)?;
    let requested_path = path;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    file_cancelled(cancel)?;
    if !path.starts_with(&root) {
        return Err(i18n::tr("right.boundary", "路径超出项目边界"));
    }
    if preview {
        if !path.is_file() {
            return Err(i18n::tr("right.special", "仅支持预览普通文件"));
        }
        file_cancelled(cancel)?;
        let mut file = fs::File::open(&path).map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        let truncated = metadata.len() > 65536;
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        while bytes.len() < 65536 {
            file_cancelled(cancel)?;
            let remaining = (65536 - bytes.len()).min(chunk.len());
            let count = file
                .read(&mut chunk[..remaining])
                .map_err(|e| e.to_string())?;
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
        let mut directory = fs::read_dir(&path).map_err(|e| e.to_string())?;
        for _ in 0..1001 {
            file_cancelled(cancel)?;
            let Some(entry) = directory.next() else {
                break;
            };
            file_cancelled(cancel)?;
            let entry = entry.map_err(|e| e.to_string())?;
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
        let path = files.preview_path.as_ref().unwrap_or(&files.directory);
        let relative = path.strip_prefix(&files.root).unwrap_or(path);
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
        let content = Rect::new(
            body.x,
            body.y + 1,
            body.width,
            body.height.saturating_sub(2),
        );
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
            }
            position = format!(
                "{} · {}–{}/{}",
                highlight::language(path, preview),
                offset + 1,
                (offset + content.height as usize).min(total),
                total
            );
            if let Some(editor) = &files.editor {
                let (row, column) = editor.position();
                position = format!(
                    "{}{} {}:{} · {}",
                    highlight::language(path, preview),
                    if editor.dirty() { " *" } else { "" },
                    row + 1,
                    column + 1,
                    i18n::tr("right.file_edit_keys", "^S保存 ^Z撤销 ^Y重做 Esc返回")
                );
                let display_column = unicode_width::UnicodeWidthStr::width(
                    editor.text[..editor.cursor]
                        .rsplit('\n')
                        .next()
                        .unwrap_or("")
                        .replace('\t', "    ")
                        .as_str(),
                );
                if self.focused
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
                let style = if index == files.selected || layout::hovered(rect, model.pointer) {
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
