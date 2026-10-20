// SPDX-License-Identifier: GPL-3.0-or-later
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use unicode_width::UnicodeWidthChar;

#[derive(Clone)]
struct Snapshot {
    text: String,
    cursor: usize,
}

pub(super) struct Editor {
    pub(super) text: String,
    pub(super) cursor: usize,
    path: PathBuf,
    canonical: PathBuf,
    original: Vec<u8>,
    metadata: fs::Metadata,
    newline: &'static str,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    preferred_column: Option<usize>,
    selection_anchor: Option<usize>,
    pub(super) search: String,
    pub(super) search_active: bool,
    pub(super) replace_mode: bool,
    pub(super) goto_mode: bool,
    pub(super) replacement: String,
    pub(super) replacement_focused: bool,
}
impl Editor {
    pub(super) fn clone_state(&self) -> Self {
        Self {
            text: self.text.clone(),
            cursor: self.cursor,
            path: self.path.clone(),
            canonical: self.canonical.clone(),
            original: self.original.clone(),
            metadata: self.metadata.clone(),
            newline: self.newline,
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            preferred_column: self.preferred_column,
            selection_anchor: self.selection_anchor,
            search: self.search.clone(),
            search_active: self.search_active,
            replace_mode: self.replace_mode,
            goto_mode: self.goto_mode,
            replacement: self.replacement.clone(),
            replacement_focused: self.replacement_focused,
        }
    }
    pub(super) fn new(path: &Path, bytes: Vec<u8>, metadata: fs::Metadata) -> Result<Self, String> {
        let text = String::from_utf8(bytes.clone()).map_err(|e| e.to_string())?;
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        Ok(Self {
            text,
            cursor: 0,
            path: path.into(),
            canonical: path.canonicalize().map_err(|e| e.to_string())?,
            original: bytes,
            metadata,
            newline,
            undo: vec![],
            redo: vec![],
            preferred_column: None,
            selection_anchor: None,
            search: String::new(),
            search_active: false,
            replace_mode: false,
            goto_mode: false,
            replacement: String::new(),
            replacement_focused: false,
        })
    }
    pub(super) fn dirty(&self) -> bool {
        self.text.as_bytes() != self.original
    }
    pub(super) fn selection(&self) -> Option<std::ops::Range<usize>> {
        self.selection_anchor
            .filter(|anchor| *anchor != self.cursor)
            .map(|anchor| anchor.min(self.cursor)..anchor.max(self.cursor))
    }
    pub(super) fn selected_text(&self) -> Option<&str> {
        self.selection().map(|range| &self.text[range])
    }
    pub(super) fn cut(&mut self) -> Option<String> {
        let range = self.selection()?;
        let text = self.text[range.clone()].to_owned();
        self.checkpoint();
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.selection_anchor = None;
        Some(text)
    }
    pub(super) fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.text.len();
    }
    pub(super) fn begin_mouse_selection(&mut self, row: usize, column: usize, extend: bool) {
        let anchor = self.selection_anchor.unwrap_or(self.cursor);
        self.move_to_display_position(row, column);
        self.selection_anchor = Some(if extend { anchor } else { self.cursor });
    }
    pub(super) fn search_position(&self) -> (usize, usize) {
        if self.search.is_empty() {
            return (0, 0);
        }
        let matches: Vec<_> = self
            .text
            .match_indices(&self.search)
            .map(|(i, _)| i)
            .collect();
        let current = self
            .selection()
            .and_then(|range| matches.iter().position(|i| *i == range.start))
            .map_or(0, |i| i + 1);
        (current, matches.len())
    }
    pub(super) fn find_next(&mut self, backwards: bool) -> bool {
        if self.search.is_empty() {
            return false;
        }
        let found = if backwards {
            self.text[..self.selection().map_or(self.cursor, |range| range.start)]
                .rfind(&self.search)
                .or_else(|| self.text.rfind(&self.search))
        } else {
            self.text[self.cursor..]
                .find(&self.search)
                .map(|i| self.cursor + i)
                .or_else(|| self.text.find(&self.search))
        };
        if let Some(start) = found {
            self.selection_anchor = Some(start);
            self.cursor = start + self.search.len();
            self.search_active = false;
            true
        } else {
            false
        }
    }
    pub(super) fn replace_all(&mut self, replacement: &str) -> usize {
        if self.search.is_empty() {
            return 0;
        }
        let count = self.text.matches(&self.search).count();
        if count > 0 {
            self.checkpoint();
            self.text = self.text.replace(&self.search, replacement);
            self.cursor = self.cursor.min(self.text.len());
            while !self.text.is_char_boundary(self.cursor) {
                self.cursor -= 1;
            }
            self.selection_anchor = None;
        }
        count
    }
    pub(super) fn replace_current(&mut self, replacement: &str) -> bool {
        let matches = self
            .selection()
            .is_some_and(|range| self.text[range] == self.search);
        if !matches && !self.find_next(false) {
            return false;
        }
        self.insert(replacement);
        if replacement.is_empty() {
            self.cut();
        }
        self.find_next(false);
        true
    }
    pub(super) fn goto_line(&mut self, line: usize) {
        let mut offset = 0usize;
        for (index, value) in self.text.split_inclusive('\n').enumerate() {
            if index + 1 >= line.max(1) {
                break;
            }
            offset += value.len();
        }
        self.cursor = offset.min(self.text.len());
        self.selection_anchor = None;
    }
    pub(super) fn indent_lines(&mut self, outdent: bool) {
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        let start = self.text[..range.start].rfind('\n').map_or(0, |i| i + 1);
        let end = self.text[range.end..]
            .find('\n')
            .map_or(self.text.len(), |i| range.end + i);
        self.checkpoint();
        let selected = self.text[start..end].to_owned();
        let lines = selected
            .split_inclusive('\n')
            .map(|line| {
                if outdent {
                    line.strip_prefix("\t")
                        .or_else(|| line.strip_prefix("    "))
                        .unwrap_or(line)
                        .to_owned()
                } else {
                    format!("    {line}")
                }
            })
            .collect::<String>();
        self.text.replace_range(start..end, &lines);
        self.cursor = self.cursor.min(self.text.len());
        self.selection_anchor = None;
    }
    pub(super) fn position(&self) -> (usize, usize) {
        let prefix = &self.text[..self.cursor];
        (
            prefix.bytes().filter(|b| *b == b'\n').count(),
            prefix.rsplit('\n').next().unwrap_or("").chars().count(),
        )
    }

    pub(super) fn move_to_display_position(&mut self, row: usize, column: usize) {
        let mut start = 0;
        for (line_index, line) in self.text.split('\n').enumerate() {
            if line_index == row {
                let line = line.strip_suffix('\r').unwrap_or(line);
                let mut display = 0;
                for (byte, ch) in line.char_indices() {
                    if display >= column {
                        self.cursor = start + byte;
                        self.preferred_column = None;
                        return;
                    }
                    let width = if ch == '\t' {
                        4 - display % 4
                    } else {
                        ch.width().unwrap_or(0)
                    };
                    if column < display + width {
                        self.cursor = start + byte;
                        self.preferred_column = None;
                        return;
                    }
                    display += width;
                }
                self.cursor = start + line.len();
                self.preferred_column = None;
                return;
            }
            start += line.len() + 1;
        }
        self.cursor = self.text.len();
        self.preferred_column = None;
    }
    fn start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }
    fn end(&self) -> usize {
        let end = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i);
        if end > 0 && self.text.as_bytes()[end - 1] == b'\r' {
            end - 1
        } else {
            end
        }
    }
    fn word_left(&self) -> usize {
        let mut at = self.cursor;
        let before = &self.text[..at];
        while let Some((index, ch)) = before[..at].char_indices().next_back() {
            if !ch.is_whitespace() {
                break;
            }
            at = index;
        }
        while let Some((index, ch)) = before[..at].char_indices().next_back() {
            if ch.is_whitespace() {
                break;
            }
            at = index;
        }
        at
    }
    fn word_right(&self) -> usize {
        let mut at = self.cursor;
        while at < self.text.len() {
            let ch = self.text[at..].chars().next().unwrap();
            if !ch.is_whitespace() {
                break;
            }
            at += ch.len_utf8();
        }
        while at < self.text.len() {
            let ch = self.text[at..].chars().next().unwrap();
            if ch.is_whitespace() {
                break;
            }
            at += ch.len_utf8();
        }
        at
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
        }
    }
    fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 128 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.preferred_column = None;
    }
    pub(super) fn insert(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.checkpoint();
        let normalized = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', self.newline);
        if let Some(range) = self.selection() {
            self.text.replace_range(range.clone(), "");
            self.cursor = range.start;
            self.selection_anchor = None;
        }
        self.text.insert_str(self.cursor, &normalized);
        self.cursor += normalized.len();
    }
    fn previous(&self) -> usize {
        if self.text[..self.cursor].ends_with("\r\n") {
            self.cursor - 2
        } else {
            self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i)
        }
    }
    fn next(&self) -> usize {
        if self.text[self.cursor..].starts_with("\r\n") {
            self.cursor + 2
        } else {
            self.cursor
                + self.text[self.cursor..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8)
        }
    }
    fn vertical(&mut self, delta: isize) {
        let (row, _) = self.position();
        let line_start = self.start();
        let line = &self.text[line_start..self.end()];
        let column = line
            .char_indices()
            .take_while(|(i, _)| line_start + i < self.cursor)
            .map(|(_, ch)| {
                if ch == '\t' {
                    4
                } else {
                    ch.width().unwrap_or(0)
                }
            })
            .sum();
        let wanted = *self.preferred_column.get_or_insert(column);
        let target = row.saturating_add_signed(delta);
        let mut start = 0;
        for (index, line) in self.text.split('\n').enumerate() {
            if index == target {
                let line = line.strip_suffix('\r').unwrap_or(line);
                let mut display = 0;
                self.cursor = start + line.len();
                for (byte, ch) in line.char_indices() {
                    if display >= wanted {
                        self.cursor = start + byte;
                        break;
                    }
                    let width = if ch == '\t' {
                        4 - display % 4
                    } else {
                        ch.width().unwrap_or(0)
                    };
                    if wanted < display + width {
                        self.cursor = start + byte;
                        break;
                    }
                    display += width;
                }
                return;
            }
            start += line.len() + 1;
        }
    }
    pub(super) fn key(&mut self, key: KeyEvent) {
        if matches!(
            key.code,
            KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
        ) {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                self.selection_anchor.get_or_insert(self.cursor);
            } else {
                self.selection_anchor = None;
            }
        }
        if matches!(key.code, KeyCode::Backspace | KeyCode::Delete) && self.selection().is_some() {
            self.cut();
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('a') => {
                    self.select_all();
                    return;
                }
                KeyCode::Char('f' | 'h' | 'g') => {
                    self.search_active = true;
                    self.replace_mode = key.code == KeyCode::Char('h');
                    self.goto_mode = key.code == KeyCode::Char('g');
                    self.replacement_focused = false;
                    if self.goto_mode {
                        self.search.clear();
                    }
                    return;
                }
                KeyCode::Home => {
                    self.cursor = 0;
                    self.preferred_column = None;
                    return;
                }
                KeyCode::End => {
                    self.cursor = self.text.len();
                    self.preferred_column = None;
                    return;
                }
                KeyCode::Left => {
                    self.cursor = self.word_left();
                    self.preferred_column = None;
                    return;
                }
                KeyCode::Right => {
                    self.cursor = self.word_right();
                    self.preferred_column = None;
                    return;
                }
                _ => {}
            }
            let redo = key.code == KeyCode::Char('y')
                || (key.code == KeyCode::Char('z') && key.modifiers.contains(KeyModifiers::SHIFT));
            if redo {
                if let Some(snapshot) = self.redo.pop() {
                    self.undo.push(self.snapshot());
                    self.text = snapshot.text;
                    self.cursor = snapshot.cursor;
                    self.selection_anchor = None;
                }
            } else if key.code == KeyCode::Char('z') {
                if let Some(snapshot) = self.undo.pop() {
                    self.redo.push(self.snapshot());
                    self.text = snapshot.text;
                    self.cursor = snapshot.cursor;
                    self.selection_anchor = None;
                }
            }
            self.preferred_column = None;
            return;
        }
        match key.code {
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.select_all()
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_active = true;
                self.replace_mode = false;
                self.goto_mode = false;
            }
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_active = true;
                self.replace_mode = true;
                self.goto_mode = false;
            }
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_active = true;
                self.goto_mode = true;
                self.replace_mode = false;
            }
            KeyCode::F(3) => {
                self.find_next(key.modifiers.contains(KeyModifiers::SHIFT));
            }
            KeyCode::Up => self.vertical(-1),
            KeyCode::Down => self.vertical(1),
            KeyCode::Left => {
                self.cursor = self.previous();
                self.preferred_column = None;
            }
            KeyCode::Right => {
                self.cursor = self.next();
                self.preferred_column = None;
            }
            KeyCode::Home => {
                let start = self.start();
                let first = self.text[start..self.end()]
                    .char_indices()
                    .find(|(_, ch)| !ch.is_whitespace())
                    .map_or(start, |(byte, _)| start + byte);
                self.cursor = if self.cursor == first { start } else { first };
                self.preferred_column = None;
            }
            KeyCode::End => {
                self.cursor = self.end();
                self.preferred_column = None;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = if key.modifiers.contains(KeyModifiers::ALT) {
                    self.word_left()
                } else {
                    self.previous()
                };
                self.checkpoint();
                self.text.replace_range(previous..self.cursor, "");
                self.cursor = previous;
            }
            KeyCode::Delete if self.cursor < self.text.len() => {
                let next = if key.modifiers.contains(KeyModifiers::ALT) {
                    self.word_right()
                } else {
                    self.next()
                };
                self.checkpoint();
                self.text.replace_range(self.cursor..next, "");
            }
            KeyCode::Enter => {
                let prefix = &self.text[self.start()..self.cursor];
                let indent: String = prefix
                    .chars()
                    .take_while(|ch| matches!(ch, ' ' | '\t'))
                    .collect();
                self.insert(&format!("\n{indent}"));
            }
            KeyCode::BackTab => self.indent_lines(true),
            KeyCode::Tab if self.selection().is_some() => self.indent_lines(false),
            KeyCode::Tab => self.insert("\t"),
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SUPER) =>
            {
                self.insert(&c.to_string())
            }
            _ => {}
        }
    }
    pub(super) fn check(&self, root: &Path) -> Result<(), String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let path = self.path.canonicalize().map_err(|e| e.to_string())?;
        if path != self.canonical || !path.starts_with(root) {
            return Err("路径已改变或超出项目边界，未保存".into());
        }
        let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.dev() != self.metadata.dev() || metadata.ino() != self.metadata.ino() {
                return Err("文件已被外部替换，未保存".into());
            }
        }
        if metadata.modified().ok() != self.metadata.modified().ok()
            || metadata.permissions() != self.metadata.permissions()
            || fs::read(&path).map_err(|e| e.to_string())? != self.original
        {
            return Err("文件已被外部修改，未保存；请保留草稿后重新打开".into());
        }
        Ok(())
    }
    pub(super) fn save(&mut self, root: &Path) -> Result<(), String> {
        self.check(root)?;
        if !self.dirty() {
            return Ok(());
        }
        let parent = self.canonical.parent().ok_or("文件没有父目录")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temp.write_all(self.text.as_bytes())
            .map_err(|e| e.to_string())?;
        temp.as_file()
            .set_permissions(self.metadata.permissions())
            .map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        self.check(root)?;
        temp.persist(&self.canonical).map_err(|e| e.to_string())?;
        self.original = self.text.as_bytes().to_vec();
        self.metadata = fs::metadata(&self.canonical).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn editor(text: &str) -> (tempfile::TempDir, Editor) {
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
        let (dir, mut e) = editor("中🙂\r\nlast");
        key(&mut e, KeyCode::End);
        key(&mut e, KeyCode::Backspace);
        assert_eq!(e.text, "中\r\nlast");
        e.insert("é\n新");
        assert_eq!(e.text, "中é\r\n新\r\nlast");
        e.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(e.text, "中\r\nlast");
        e.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        e.save(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("file")).unwrap(), e.text);
        assert!(!e.dirty());
    }
    #[test]
    fn selection_typing_and_search_shortcuts_work() {
        let (_, mut e) = editor("中文 hello");
        e.key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
        assert_eq!(e.selected_text(), Some("中"));
        e.insert("新");
        assert_eq!(e.text, "新文 hello");
        e.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(e.selected_text(), Some("新文 hello"));
        e.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert!(e.text.is_empty());
        e.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
        assert!(e.search_active && e.replace_mode);
    }
    #[test]
    fn mouse_drag_and_reverse_search_preserve_unicode_selection() {
        let (_, mut e) = editor("甲乙 甲乙");
        e.begin_mouse_selection(0, 0, false);
        e.move_to_display_position(0, 4);
        assert_eq!(e.selected_text(), Some("甲乙"));
        e.search = "甲乙".into();
        assert!(e.find_next(false));
        assert_eq!(e.search_position(), (2, 2));
        assert!(e.find_next(true));
        assert_eq!(e.search_position(), (1, 2));
        assert!(e.find_next(true));
        assert_eq!(e.search_position(), (2, 2));
    }
    #[test]
    fn smart_home_and_enter_preserve_indentation() {
        let (_, mut e) = editor("    中文");
        e.cursor = e.text.len();
        key(&mut e, KeyCode::Home);
        assert_eq!(e.cursor, 4);
        key(&mut e, KeyCode::Home);
        assert_eq!(e.cursor, 0);
        key(&mut e, KeyCode::End);
        key(&mut e, KeyCode::Enter);
        assert_eq!(e.text, "    中文\n    ");
        e.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(e.text, "    中文");
    }
    #[test]
    fn single_replace_changes_one_match_and_selects_next() {
        let (_, mut e) = editor("甲 甲 甲");
        e.search = "甲".into();
        assert!(e.replace_current("乙"));
        assert_eq!(e.text, "乙 甲 甲");
        assert_eq!(e.search_position(), (1, 2));
        e.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(e.text, "甲 甲 甲");
    }
    #[test]
    fn replace_all_keeps_cursor_at_utf8_boundary_and_is_undoable() {
        let (_, mut e) = editor("a甲甲");
        e.cursor = 1;
        e.search = "a".into();
        assert_eq!(e.replace_all(""), 1);
        assert!(e.text.is_char_boundary(e.cursor));
        e.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(e.text, "a甲甲");
    }
    #[test]
    fn mouse_position_maps_wide_characters_and_tabs_to_utf8_boundaries() {
        let (_, mut e) = editor("甲🙂\tZ\nnext");
        e.move_to_display_position(0, 2);
        assert_eq!(&e.text[e.cursor..], "🙂\tZ\nnext");
        e.move_to_display_position(0, 4);
        assert_eq!(&e.text[e.cursor..], "\tZ\nnext");
        e.move_to_display_position(0, 8);
        assert_eq!(&e.text[e.cursor..], "Z\nnext");
        e.move_to_display_position(1, 99);
        assert_eq!(e.cursor, e.text.len());
    }

    #[test]
    fn vertical_home_end_and_delete() {
        let (_, mut e) = editor("中文\na\nthree");
        key(&mut e, KeyCode::End);
        key(&mut e, KeyCode::Down);
        assert_eq!(e.position(), (1, 1));
        key(&mut e, KeyCode::Down);
        assert_eq!(e.position(), (2, 4));
        key(&mut e, KeyCode::Home);
        key(&mut e, KeyCode::Delete);
        assert!(e.text.ends_with("hree"));
    }
    #[test]
    fn control_word_and_document_navigation_preserve_unicode_boundaries() {
        let (_, mut e) = editor("alpha 中文 beta\nlast");
        key(&mut e, KeyCode::End);
        e.key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(&e.text[e.cursor..], "beta\nlast");
        e.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
        assert_eq!(e.cursor, 0);
        e.key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL));
        assert_eq!(e.cursor, e.text.len());
    }
    #[test]
    fn vertical_navigation_keeps_display_column_across_wide_text() {
        let (_, mut e) = editor("中a\n123456");
        e.move_to_display_position(0, 3);
        key(&mut e, KeyCode::Down);
        assert_eq!(&e.text[e.cursor..], "456");
    }
    #[test]
    fn conflict_and_boundary_never_overwrite() {
        let (dir, mut e) = editor("original");
        e.insert("draft");
        let other = tempfile::tempdir().unwrap();
        assert!(e.save(other.path()).is_err());
        fs::write(dir.path().join("file"), "external").unwrap();
        assert!(e.save(dir.path()).is_err());
        assert!(e.dirty());
        assert_eq!(
            fs::read_to_string(dir.path().join("file")).unwrap(),
            "external"
        );
    }
    #[cfg(unix)]
    #[test]
    fn atomic_save_preserves_permissions_and_symlink_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (dir, _) = editor("original");
        let path = dir.path().join("file");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        let mut e = Editor::new(
            &link,
            fs::read(&path).unwrap(),
            fs::metadata(&path).unwrap(),
        )
        .unwrap();
        e.insert("new");
        e.save(dir.path()).unwrap();
        assert!(fs::symlink_metadata(link).unwrap().is_symlink());
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
