// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Right-panel file editor: document, viewport highlighting, find/replace.

pub(super) mod document;
mod find;
pub(super) mod highlight;
mod preview;
pub(super) mod render;

pub(super) use preview::Preview;

use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use document::Document;
use find::{FindFocus, FindState};
use highlight::HighlightCache;
use ratatui::{
    style::{Modifier, Style},
    text::Line,
};
use std::{
    cell::{Cell, RefCell},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FindAction {
    Handled,
    Close,
    Next,
    Prev,
    Replace,
    ReplaceAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GotoAction {
    Handled,
    Close,
    Jump,
}

pub(super) struct FindView<'a> {
    pub(super) query: &'a str,
    pub(super) replacement: &'a str,
    pub(super) replace: bool,
    pub(super) focus_replace: bool,
    pub(super) matches: usize,
    pub(super) current: Option<usize>,
    pub(super) case_sensitive: bool,
    pub(super) error: Option<&'a str>,
}

struct WarmJob {
    rx: mpsc::Receiver<(Vec<highlight::WarmCheckpoint>, bool)>,
    stop: Arc<AtomicBool>,
    version: u64,
    dark: bool,
}

struct BracketCache {
    version: u64,
    cursor: usize,
    pair: Option<(usize, usize)>,
}

pub(super) struct Editor {
    doc: Document,
    path: PathBuf,
    canonical: PathBuf,
    original: Vec<u8>,
    metadata: fs::Metadata,
    find: FindState,
    goto: Option<String>,
    highlight: RefCell<HighlightCache>,
    warm: RefCell<Option<WarmJob>>,
    bracket: RefCell<Option<BracketCache>>,
    viewport_rows: Cell<usize>,
    last_processed: Cell<usize>,
}

impl Editor {
    pub(super) fn new(path: &Path, bytes: Vec<u8>, metadata: fs::Metadata) -> Result<Self, String> {
        let text = String::from_utf8(bytes.clone()).map_err(|error| error.to_string())?;
        let language = highlight::language(path, &text);
        let doc = Document::new(text, language == "Python");
        let highlight = HighlightCache::new(path, doc.text(), Theme::current().is_dark());
        let editor = Self {
            doc,
            path: path.into(),
            canonical: path.canonicalize().map_err(|error| error.to_string())?,
            original: bytes,
            metadata,
            find: FindState::default(),
            goto: None,
            highlight: RefCell::new(highlight),
            warm: RefCell::new(None),
            bracket: RefCell::new(None),
            viewport_rows: Cell::new(10),
            last_processed: Cell::new(0),
        };
        editor.start_warm();
        Ok(editor)
    }

    fn start_warm(&self) {
        if self.doc.line_count() <= highlight::CHECKPOINT_LINES * 2 {
            return;
        }
        let text = self.doc.text().to_owned();
        let starts = self.doc.line_starts().to_vec();
        let syntax = highlight::syntax(&self.path, &text);
        let dark = Theme::current().is_dark();
        let version = self.doc.version();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("blora-editor-warm".into())
            .spawn(move || {
                highlight::compute_checkpoints(&text, &starts, syntax, dark, &mut |chunk, done| {
                    if worker_stop.load(Ordering::Relaxed) {
                        return false;
                    }
                    tx.send((chunk, done)).is_ok()
                });
            });
        if spawned.is_ok() {
            *self.warm.borrow_mut() = Some(WarmJob {
                rx,
                stop,
                version,
                dark,
            });
        }
    }

    fn poll_warm(&self) {
        let mut job = self.warm.borrow_mut();
        let Some(active) = job.as_mut() else {
            return;
        };
        let version_matches = self.doc.version() == active.version;
        let dark_matches = self.highlight.borrow().is_dark() == active.dark;
        let mut finished = false;
        loop {
            match active.rx.try_recv() {
                Ok((chunk, done)) => {
                    if version_matches && dark_matches {
                        self.highlight.borrow_mut().add_checkpoints(chunk);
                    } else {
                        finished = true;
                    }
                    if done {
                        finished = true;
                        break;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            active.stop.store(true, Ordering::Relaxed);
            *job = None;
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn eol(&self) -> &'static str {
        self.doc.newline_style()
    }

    pub(super) fn text(&self) -> &str {
        self.doc.text()
    }

    pub(super) fn cursor(&self) -> usize {
        self.doc.cursor()
    }

    pub(super) fn dirty(&self) -> bool {
        self.doc.is_dirty()
    }

    pub(super) fn position(&self) -> (usize, usize) {
        self.doc.position()
    }

    pub(super) fn line_count(&self) -> usize {
        self.doc.line_count()
    }

    pub(super) fn line_start(&self, line: usize) -> usize {
        self.doc.line_start(line)
    }

    pub(super) fn display_column(&self, byte: usize) -> usize {
        self.doc.display_column(byte)
    }

    pub(super) fn selection(&self) -> Option<(usize, usize)> {
        self.doc.selection()
    }

    pub(super) fn word_at(&self, byte: usize) -> Option<(usize, usize)> {
        self.doc.word_at(byte)
    }

    pub(super) fn line_bounds_at(&self, byte: usize) -> (usize, usize) {
        self.doc.line_bounds_at(byte)
    }

    pub(super) fn move_to_display_position(&mut self, line: usize, column: usize) {
        self.doc.move_to_display_position(line, column);
    }

    #[cfg(test)]
    pub(super) fn set_cursor(&mut self, byte: usize) {
        self.doc.set_cursor(byte);
    }

    pub(super) fn byte_at_display_position(&self, line: usize, column: usize) -> usize {
        self.doc.byte_at_display_column(line, column)
    }

    pub(super) fn select(&mut self, start: usize, end: usize) {
        self.doc.select(start, end);
    }

    pub(super) fn insert(&mut self, text: &str) {
        let normalized = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', self.newline());
        self.doc.insert_text(&normalized);
    }

    fn newline(&self) -> &'static str {
        if self.doc.text().contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        }
    }

    pub(super) fn set_viewport_rows(&self, rows: usize) {
        self.viewport_rows.set(rows.max(1));
    }

    pub(super) fn language(&self) -> &'static str {
        self.highlight.borrow().language()
    }

    pub(super) fn key(&mut self, key: KeyEvent) {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if control {
            match key.code {
                KeyCode::Char('z') if !shift => {
                    self.doc.undo();
                }
                KeyCode::Char('z') | KeyCode::Char('y') => {
                    self.doc.redo();
                }
                KeyCode::Char('a') => self.doc.select_all(),
                KeyCode::Left => self.doc.word_left(shift),
                KeyCode::Right => self.doc.word_right(shift),
                KeyCode::Home => self.doc.doc_start(shift),
                KeyCode::End => self.doc.doc_end(shift),
                KeyCode::Backspace => self.doc.delete_word_left(),
                KeyCode::Delete => self.doc.delete_word_right(),
                _ => {}
            }
            return;
        }
        if alt {
            match key.code {
                KeyCode::Up => {
                    self.doc.move_lines(-1);
                }
                KeyCode::Down => {
                    self.doc.move_lines(1);
                }
                _ => {}
            }
            return;
        }
        let rows = self.viewport_rows.get();
        match key.code {
            KeyCode::Up => self.doc.up(shift),
            KeyCode::Down => self.doc.down(shift),
            KeyCode::Left => self.doc.left(shift),
            KeyCode::Right => self.doc.right(shift),
            KeyCode::Home => self.doc.home(shift, true),
            KeyCode::End => self.doc.end(shift),
            KeyCode::PageUp => self.doc.page(-1, rows, shift),
            KeyCode::PageDown => self.doc.page(1, rows, shift),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.doc.delete(),
            KeyCode::Enter => self.doc.newline(),
            KeyCode::Tab if shift => self.doc.outdent(),
            KeyCode::Tab => self.doc.tab(),
            KeyCode::BackTab => self.doc.outdent(),
            KeyCode::Char(ch) if !control && !alt => self.type_char(ch),
            _ => {}
        }
    }

    fn next_char(&self) -> Option<char> {
        self.doc.text()[self.doc.cursor()..].chars().next()
    }

    fn previous_char(&self) -> Option<char> {
        self.doc.text()[..self.doc.cursor()].chars().next_back()
    }

    fn type_char(&mut self, ch: char) {
        if let Some(close) = pair_close(ch) {
            if self.doc.has_selection() {
                self.doc.wrap_selection(&ch.to_string(), &close.to_string());
                return;
            }
            if self.next_char() == Some(close) {
                self.doc.right(false);
                return;
            }
            let inside_word = self
                .previous_char()
                .is_some_and(|previous| previous.is_alphanumeric() || previous == '_');
            if inside_word && (ch == '\'' || ch == '"') {
                self.doc.insert_typed(&ch.to_string());
                return;
            }
            if self.next_char().is_some_and(|next| next.is_alphanumeric()) {
                self.doc.insert_typed(&ch.to_string());
                return;
            }
            self.doc.insert_pair(ch, close);
            return;
        }
        if matches!(ch, ')' | ']' | '}') && self.next_char() == Some(ch) {
            self.doc.right(false);
            return;
        }
        self.doc.insert_typed(&ch.to_string());
    }

    fn backspace(&mut self) {
        if !self.doc.has_selection()
            && let (Some(close), Some(open)) = (self.next_char(), self.previous_char())
            && pair_close(open) == Some(close)
        {
            self.doc.delete_pair_before();
            return;
        }
        self.doc.backspace();
    }

    pub(super) fn cut(&mut self) -> String {
        if let Some(text) = self.doc.selected_text() {
            let text = text.to_owned();
            self.doc.delete_selection();
            return text;
        }
        let (start, end) = self.current_line_span();
        let text = self.doc.text()[start..end].to_owned();
        self.doc.select(start, end);
        self.doc.delete_selection();
        text
    }

    fn current_line_span(&self) -> (usize, usize) {
        let line = self.doc.line_of(self.doc.cursor());
        let start = self.doc.line_start(line);
        let end = self.doc.line_end(line);
        let rest = &self.doc.text()[end..];
        let end = if rest.starts_with("\r\n") {
            end + 2
        } else if rest.starts_with('\n') {
            end + 1
        } else {
            end
        };
        (start, end)
    }

    pub(super) fn copy_text(&self) -> String {
        if let Some(text) = self.doc.selected_text() {
            return text.to_owned();
        }
        let (start, end) = self.current_line_span();
        self.doc.text()[start..end].to_owned()
    }

    pub(super) fn find_view(&self) -> Option<FindView<'_>> {
        self.find.open.then(|| FindView {
            query: &self.find.query,
            replacement: &self.find.replacement,
            replace: self.find.replace,
            focus_replace: self.find.focus == Some(FindFocus::Replacement),
            matches: self.find.matches.len(),
            current: self.find.current,
            case_sensitive: self.find.case_sensitive(),
            error: self.find.error.as_deref(),
        })
    }

    pub(super) fn find_open(&mut self, replace: bool) {
        let initial = self.doc.selected_text().unwrap_or("").to_owned();
        if replace {
            self.find.open_replace(&initial);
        } else {
            self.find.open_find(&initial);
        }
        self.refresh_find();
    }

    pub(super) fn discard_changes(&mut self) {
        self.doc.mark_saved();
    }

    pub(super) fn find_close(&mut self) {
        self.find.close();
    }

    fn refresh_find(&mut self) {
        let cursor = self
            .doc
            .selection()
            .map_or(self.doc.cursor(), |(start, _)| start);
        self.find.refresh(self.doc.text(), cursor);
        if let Some(range) = self.find.current() {
            self.doc.select(range.start, range.end);
        }
    }

    pub(super) fn find_key(&mut self, key: KeyEvent) -> FindAction {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => {
                self.find_close();
                FindAction::Close
            }
            KeyCode::Tab => {
                self.find.toggle_focus();
                FindAction::Handled
            }
            KeyCode::Enter if control && alt => {
                self.replace_all();
                FindAction::ReplaceAll
            }
            KeyCode::Enter if alt => {
                self.replace_current();
                FindAction::Replace
            }
            KeyCode::Enter if shift => {
                self.find.step(false);
                self.refresh_selection();
                FindAction::Prev
            }
            KeyCode::Enter => {
                self.find.step(true);
                self.refresh_selection();
                FindAction::Next
            }
            KeyCode::F(3) => {
                self.find.step(!shift);
                self.refresh_selection();
                FindAction::Next
            }
            KeyCode::Backspace => {
                if let Some(field) = self.find.focus_mut() {
                    field.pop();
                }
                self.refresh_find();
                FindAction::Handled
            }
            KeyCode::Char(ch) if !control && !alt => {
                if let Some(field) = self.find.focus_mut() {
                    field.push(ch);
                }
                self.refresh_find();
                FindAction::Handled
            }
            _ => FindAction::Handled,
        }
    }

    pub(super) fn find_step(&mut self, forward: bool) {
        self.find.step(forward);
        self.refresh_selection();
    }

    pub(super) fn find_paste(&mut self, text: &str) {
        let text = text.replace(['\r', '\n'], " ");
        if let Some(field) = self.find.focus_mut() {
            field.push_str(&text);
        }
        self.refresh_find();
    }

    pub(super) fn goto_paste(&mut self, text: &str) {
        if let Some(buffer) = self.goto.as_mut() {
            buffer.extend(text.chars().filter(char::is_ascii_digit));
        }
    }

    fn refresh_selection(&mut self) {
        if let Some(range) = self.find.current() {
            self.doc.select(range.start, range.end);
        }
    }

    pub(super) fn find_toggle_case(&mut self) {
        self.find.toggle_case(self.doc.text());
        self.refresh_selection();
    }

    fn replace_current(&mut self) {
        let Some(range) = self.find.current() else {
            return;
        };
        let replacement = self.find.replacement.clone();
        self.doc.replace_span(range.start, range.end, &replacement);
        let cursor = self.doc.cursor();
        self.find.refresh_near(self.doc.text(), cursor);
        self.refresh_selection();
    }

    fn replace_all(&mut self) {
        let ranges = self.find.all_ranges();
        let replacement = self.find.replacement.clone();
        self.doc.replace_all(&ranges, &replacement);
        self.find.refresh(self.doc.text(), 0);
        self.refresh_selection();
    }

    pub(super) fn goto_open(&mut self) {
        self.goto = Some(String::new());
    }

    pub(super) fn goto_view(&self) -> Option<&str> {
        self.goto.as_deref()
    }

    pub(super) fn goto_key(&mut self, key: KeyEvent) -> GotoAction {
        let Some(buffer) = self.goto.as_mut() else {
            return GotoAction::Handled;
        };
        match key.code {
            KeyCode::Esc => {
                self.goto = None;
                GotoAction::Close
            }
            KeyCode::Backspace => {
                buffer.pop();
                GotoAction::Handled
            }
            KeyCode::Enter => {
                let line = buffer.trim().parse::<usize>().ok();
                self.goto = None;
                if let Some(line) = line {
                    let line = line
                        .saturating_sub(1)
                        .min(self.doc.line_count().saturating_sub(1));
                    let byte = self.doc.line_start(line);
                    self.doc.set_cursor(byte);
                    return GotoAction::Jump;
                }
                GotoAction::Close
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                buffer.push(ch);
                GotoAction::Handled
            }
            _ => GotoAction::Handled,
        }
    }

    pub(super) fn max_width(&self, dark: bool) -> usize {
        self.sync_highlight(dark);
        let mut cache = self.highlight.borrow_mut();
        cache.max_width(self.doc.text(), self.doc.line_starts())
    }

    pub(super) fn ensure_viewport(&self, first: usize, rows: usize, dark: bool) {
        self.poll_warm();
        self.sync_highlight(dark);
        let warming = self.warm.borrow().is_some();
        let mut cache = self.highlight.borrow_mut();
        let result = cache.ensure(
            self.doc.text(),
            self.doc.line_starts(),
            first,
            rows,
            warming,
        );
        self.last_processed.set(result.processed);
    }

    #[cfg(test)]
    pub(super) fn last_processed(&self) -> usize {
        self.last_processed.get()
    }

    fn sync_highlight(&self, dark: bool) {
        let mut cache = self.highlight.borrow_mut();
        if cache.is_dark() != dark {
            *cache = HighlightCache::new(&self.path, self.doc.text(), dark);
        }
        if let Some(line) = self.doc.take_dirty_from() {
            cache.invalidate_from(line);
        }
    }

    pub(super) fn render_line(
        &self,
        index: usize,
        width: usize,
        horizontal: usize,
        dark: bool,
        theme: &Theme,
    ) -> Line<'static> {
        let _ = dark;
        let plain_style = theme.fg(theme.text);
        let line = {
            let cache = self.highlight.borrow();
            cache
                .line(index)
                .cloned()
                .unwrap_or_else(|| highlight::CachedLine {
                    segments: vec![highlight::Segment {
                        style: Style::default(),
                        text: self.doc.line_text(index).to_owned(),
                    }],
                    plain: true,
                })
        };
        let mut overlays = Vec::new();
        if let Some(pair) = self.bracket_pair() {
            let style = Style::default()
                .bg(theme.hairline)
                .add_modifier(Modifier::BOLD);
            if let Some((from, to)) = self.column_range(index, pair.0, pair.0.saturating_add(1)) {
                overlays.push(render::Overlay { from, to, style });
            }
            if let Some((from, to)) = self.column_range(index, pair.1, pair.1.saturating_add(1)) {
                overlays.push(render::Overlay { from, to, style });
            }
        }
        if self.find.open {
            let line_start = self.doc.line_start(index);
            let line_end = self.doc.line_end(index);
            let first = self
                .find
                .matches
                .partition_point(|(_, end)| *end <= line_start);
            let current = self.find.current;
            for (match_index, (start, end)) in self.find.matches.iter().enumerate().skip(first) {
                if *start > line_end {
                    break;
                }
                if let Some((from, to)) = self.column_range(index, *start, *end) {
                    let style = if current == Some(match_index) {
                        Style::default().bg(theme.amber).fg(theme.bg)
                    } else {
                        Style::default().bg(theme.bg_select)
                    };
                    overlays.push(render::Overlay { from, to, style });
                }
            }
        }
        if let Some((start, end)) = self.doc.selection()
            && let Some((from, to)) = self.column_range(index, start, end)
        {
            overlays.push(render::Overlay {
                from,
                to,
                style: Style::default().bg(theme.bg_select),
            });
        }
        let base = if index == self.doc.line_of(self.doc.cursor()) {
            Style::default().bg(theme.bg_raised)
        } else {
            Style::default()
        };
        render::render(&line, plain_style, horizontal, width, base, &overlays)
    }

    fn column_range(&self, line: usize, start: usize, end: usize) -> Option<(usize, usize)> {
        let line_start = self.doc.line_start(line);
        let line_end = self.doc.line_end(line);
        if end <= line_start || start > line_end {
            return None;
        }
        let from = self.doc.display_column(start.max(line_start));
        let to = if end > line_end {
            usize::MAX
        } else {
            self.doc.display_column(end.min(line_end))
        };
        (to > from).then_some((from, to))
    }

    fn bracket_pair(&self) -> Option<(usize, usize)> {
        let version = self.doc.version();
        let cursor = self.doc.cursor();
        let mut cache = self.bracket.borrow_mut();
        if let Some(cached) = cache.as_ref()
            && cached.version == version
            && cached.cursor == cursor
        {
            return cached.pair;
        }
        let pair = self.doc.matching_bracket();
        *cache = Some(BracketCache {
            version,
            cursor,
            pair,
        });
        pair
    }

    fn check(&self, root: &Path) -> Result<(), String> {
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        let path = self
            .path
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if path != self.canonical || !path.starts_with(root) {
            return Err("路径已改变或超出项目边界，未保存".into());
        }
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.dev() != self.metadata.dev() || metadata.ino() != self.metadata.ino() {
                return Err("文件已被外部替换，未保存".into());
            }
        }
        if metadata.modified().ok() != self.metadata.modified().ok()
            || metadata.permissions() != self.metadata.permissions()
            || fs::read(&path).map_err(|error| error.to_string())? != self.original
        {
            return Err("文件已被外部修改，未保存；请保留草稿后重新打开".into());
        }
        Ok(())
    }

    pub(super) fn save(&mut self, root: &Path) -> Result<(), String> {
        self.check(root)?;
        if !self.doc.is_dirty() {
            return Ok(());
        }
        let parent = self.canonical.parent().ok_or("文件没有父目录")?;
        let mut temp =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        temp.write_all(self.doc.text().as_bytes())
            .map_err(|error| error.to_string())?;
        temp.as_file()
            .set_permissions(self.metadata.permissions())
            .map_err(|error| error.to_string())?;
        temp.as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        self.check(root)?;
        temp.persist(&self.canonical)
            .map_err(|error| error.to_string())?;
        self.original = self.doc.text().as_bytes().to_vec();
        self.metadata = fs::metadata(&self.canonical).map_err(|error| error.to_string())?;
        self.doc.mark_saved();
        Ok(())
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        if let Some(job) = self.warm.get_mut() {
            job.stop.store(true, Ordering::Relaxed);
        }
    }
}

fn pair_close(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    fn sample(text: &str) -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        fs::write(&path, text).unwrap();
        let editor = Editor::new(
            &path,
            text.as_bytes().to_vec(),
            fs::metadata(&path).unwrap(),
        )
        .unwrap();
        (dir, editor)
    }

    fn key(editor: &mut Editor, code: KeyCode) {
        editor.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn unicode_crlf_paste_and_history() {
        let (dir, mut editor) = sample("中🙂\r\nlast");
        key(&mut editor, KeyCode::End);
        key(&mut editor, KeyCode::Backspace);
        assert_eq!(editor.text(), "中\r\nlast");
        editor.insert("é\n新");
        assert_eq!(editor.text(), "中é\r\n新\r\nlast");
        editor.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.text(), "中\r\nlast");
        editor.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        editor.save(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("file")).unwrap(),
            editor.text()
        );
        assert!(!editor.dirty());
    }

    #[test]
    fn mouse_position_maps_wide_characters_and_tabs_to_utf8_boundaries() {
        let (_dir, mut editor) = sample("甲🙂\tZ\nnext");
        editor.move_to_display_position(0, 2);
        assert_eq!(&editor.text()[editor.cursor()..], "🙂\tZ\nnext");
        editor.move_to_display_position(0, 4);
        assert_eq!(&editor.text()[editor.cursor()..], "\tZ\nnext");
        editor.move_to_display_position(0, 8);
        assert_eq!(&editor.text()[editor.cursor()..], "Z\nnext");
        editor.move_to_display_position(1, 99);
        assert_eq!(editor.cursor(), editor.text().len());
    }

    #[test]
    fn vertical_home_end_and_delete() {
        let (_dir, mut editor) = sample("中文\na\nthree");
        key(&mut editor, KeyCode::End);
        key(&mut editor, KeyCode::Down);
        assert_eq!(editor.position(), (1, 1));
        key(&mut editor, KeyCode::Down);
        assert_eq!(editor.position(), (2, 4));
        key(&mut editor, KeyCode::Home);
        key(&mut editor, KeyCode::Delete);
        assert!(editor.text().ends_with("hree"));
    }

    #[test]
    fn conflict_and_boundary_never_overwrite() {
        let (dir, mut editor) = sample("original");
        editor.insert("draft");
        let other = tempfile::tempdir().unwrap();
        assert!(editor.save(other.path()).is_err());
        fs::write(dir.path().join("file"), "external").unwrap();
        assert!(editor.save(dir.path()).is_err());
        assert!(editor.dirty());
        assert_eq!(
            fs::read_to_string(dir.path().join("file")).unwrap(),
            "external"
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_preserves_permissions_and_symlink_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (dir, _) = sample("original");
        let path = dir.path().join("file");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        let mut editor = Editor::new(
            &link,
            fs::read(&path).unwrap(),
            fs::metadata(&path).unwrap(),
        )
        .unwrap();
        editor.insert("new");
        editor.save(dir.path()).unwrap();
        assert!(fs::symlink_metadata(link).unwrap().is_symlink());
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn find_replace_and_goto_work() {
        let (_dir, mut editor) = sample("one two one\nlast");
        editor.find_open(false);
        editor.find.query = "one".into();
        editor.refresh_find();
        assert_eq!(editor.find.matches.len(), 2);
        assert_eq!(editor.selection(), Some((0, 3)));
        editor.find_step(true);
        assert_eq!(editor.selection(), Some((8, 11)));
        editor.find_close();
        editor.goto_open();
        for ch in "2".chars() {
            editor.goto_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(
            editor.goto_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            GotoAction::Jump
        );
        assert_eq!(editor.position().0, 1);
    }

    #[test]
    fn replace_all_is_one_undo_step() {
        let (_dir, mut editor) = sample("one one one");
        editor.find_open(true);
        editor.find.query = "one".into();
        editor.find.replacement = "two".into();
        editor.refresh_find();
        editor.replace_all();
        assert_eq!(editor.text(), "two two two");
        editor.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.text(), "one one one");
    }

    #[test]
    fn viewport_highlight_is_bounded_and_cached() {
        let text: String = (0..2000)
            .map(|index| format!("let x = {index};\n"))
            .collect();
        let (_dir, editor) = sample(&text);
        let dark = crate::theme::Theme::current().is_dark();
        editor.ensure_viewport(0, 40, dark);
        let first = editor.last_processed();
        assert!(first > 0);
        assert!(first <= 40 + highlight::OVERSCAN + highlight::CHECKPOINT_LINES);
        editor.ensure_viewport(0, 40, dark);
        assert_eq!(editor.last_processed(), 0);
    }

    #[test]
    fn auto_pairs_wrap_selection_and_type_over() {
        let (_dir, mut editor) = sample("word");
        editor.select(0, 4);
        editor.type_char('(');
        assert_eq!(editor.text(), "(word)");
        assert_eq!(editor.selection(), Some((1, 5)));
        editor.set_cursor(5);
        editor.type_char(')');
        assert_eq!(editor.text(), "(word)");
        assert_eq!(editor.cursor(), 6);
        let (_dir, mut fresh) = sample("");
        fresh.type_char('(');
        assert_eq!(fresh.text(), "()");
        assert_eq!(fresh.cursor(), 1);
        fresh.type_char(')');
        assert_eq!(fresh.cursor(), 2);
    }
}
