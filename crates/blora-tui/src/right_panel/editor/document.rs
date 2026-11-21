// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Text buffer and editing model for the right-panel file editor.
//!
//! The document keeps a single UTF-8 string plus a byte line index. Every
//! position/row/column operation is O(log n) or O(line width); edits record
//! reverse deltas so undo does not clone the whole file.

use std::{
    cell::Cell,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthChar;

pub(super) const TAB_WIDTH: usize = 4;
const MAX_UNDO_GROUPS: usize = 256;
const UNDO_COALESCE: Duration = Duration::from_millis(500);
pub(super) const MAX_BRACKET_SCAN: usize = 200 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Indent {
    pub(super) tabs: bool,
    pub(super) width: usize,
}

impl Default for Indent {
    fn default() -> Self {
        Self {
            tabs: false,
            width: 4,
        }
    }
}

impl Indent {
    fn unit(&self) -> &'static str {
        if self.tabs { "\t" } else { "    " }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Typing,
    Backspace,
    Delete,
    Paste,
    Other,
}

#[derive(Clone, Debug)]
struct Change {
    start: usize,
    removed: String,
    inserted: String,
}

#[derive(Clone, Debug)]
struct EditGroup {
    changes: Vec<Change>,
    cursor_before: usize,
    anchor_before: Option<usize>,
    cursor_after: usize,
    anchor_after: Option<usize>,
}

#[derive(Clone)]
pub(super) struct Document {
    text: String,
    line_starts: Vec<usize>,
    cursor: usize,
    anchor: Option<usize>,
    preferred_column: Option<usize>,
    undo: Vec<EditGroup>,
    redo: Vec<EditGroup>,
    version: u64,
    dirty: bool,
    saved_hash: u64,
    dirty_from: Cell<Option<usize>>,
    indent: Indent,
    newline: &'static str,
    python: bool,
    last_edit: Option<(Instant, EditKind)>,
}

impl Document {
    pub(super) fn new(text: String, python: bool) -> Self {
        let indent = detect_indent(&text);
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let saved_hash = content_hash(&text);
        let mut document = Self {
            text,
            line_starts: Vec::new(),
            cursor: 0,
            anchor: None,
            preferred_column: None,
            undo: Vec::new(),
            redo: Vec::new(),
            version: 0,
            dirty: false,
            saved_hash,
            dirty_from: Cell::new(None),
            indent,
            newline,
            python,
            last_edit: None,
        };
        document.rebuild_lines();
        document
    }

    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(super) fn version(&self) -> u64 {
        self.version
    }

    pub(super) fn newline_style(&self) -> &'static str {
        self.newline
    }

    pub(super) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(super) fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.text.len());
        self.anchor = None;
        self.preferred_column = None;
        self.break_undo_group();
    }

    pub(super) fn select(&mut self, start: usize, end: usize) {
        let (start, end) = (start.min(self.text.len()), end.min(self.text.len()));
        if start <= end {
            self.cursor = end;
            self.anchor = (start != end).then_some(start);
        } else {
            self.cursor = end;
            self.anchor = Some(start);
        }
        self.preferred_column = None;
        self.break_undo_group();
    }

    pub(super) fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    pub(super) fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        let (start, end) = if anchor <= self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        };
        (start != end).then_some((start, end))
    }

    pub(super) fn selected_text(&self) -> Option<&str> {
        self.selection().map(|(start, end)| &self.text[start..end])
    }

    pub(super) fn select_all(&mut self) {
        self.anchor = (!self.text.is_empty()).then_some(0);
        self.cursor = self.text.len();
        self.preferred_column = None;
    }

    pub(super) fn delete_selection(&mut self) {
        if let Some((start, end)) = self.selection() {
            self.replace_range(start, end, "");
        }
    }

    pub(super) fn replace_span(&mut self, start: usize, end: usize, text: &str) {
        self.replace_range(start, end, text);
    }

    pub(super) fn replace_all(&mut self, ranges: &[(usize, usize)], replacement: &str) -> usize {
        if ranges.is_empty() {
            return 0;
        }
        let old = self.text.clone();
        let mut new = String::with_capacity(old.len());
        let mut last = 0;
        let mut count = 0;
        for (start, end) in ranges {
            if *start < last || *end > old.len() || start > end {
                continue;
            }
            new.push_str(&old[last..*start]);
            new.push_str(replacement);
            last = *end;
            count += 1;
        }
        if count == 0 {
            return 0;
        }
        new.push_str(&old[last..]);
        self.bulk_replace(0, old.len(), &old, new, 0, None, 0);
        count
    }

    pub(super) fn line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    pub(super) fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub(super) fn line_of(&self, byte: usize) -> usize {
        let byte = byte.min(self.text.len());
        self.line_starts
            .partition_point(|start| *start <= byte)
            .saturating_sub(1)
    }

    pub(super) fn line_start(&self, line: usize) -> usize {
        self.line_starts
            .get(line)
            .copied()
            .unwrap_or(self.text.len())
    }

    fn line_end_full(&self, line: usize) -> usize {
        self.line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len())
    }

    pub(super) fn line_end(&self, line: usize) -> usize {
        if line >= self.line_starts.len() {
            return self.text.len();
        }
        let mut end = self.line_end_full(line);
        if end > self.line_starts[line]
            && self.text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n')
        {
            end -= 1;
        }
        if end > self.line_starts[line]
            && self.text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\r')
        {
            end -= 1;
        }
        end
    }

    pub(super) fn line_text(&self, line: usize) -> &str {
        if line >= self.line_starts.len() {
            return "";
        }
        &self.text[self.line_start(line)..self.line_end(line)]
    }

    pub(super) fn row_col(&self, byte: usize) -> (usize, usize) {
        let line = self.line_of(byte);
        let start = self.line_start(line);
        (
            line,
            self.text[start..byte.min(self.text.len())].chars().count(),
        )
    }

    pub(super) fn position(&self) -> (usize, usize) {
        self.row_col(self.cursor)
    }

    pub(super) fn display_column(&self, byte: usize) -> usize {
        let line = self.line_of(byte);
        let start = self.line_start(line);
        let end = byte.min(self.line_end_full(line));
        display_width(&self.text[start..end])
    }

    pub(super) fn byte_at_display_column(&self, line: usize, column: usize) -> usize {
        let start = self.line_start(line);
        let end = self.line_end(line);
        let mut display = 0;
        for (offset, ch) in self.text[start..end].char_indices() {
            if offset > 0 && display >= column {
                return start + offset;
            }
            let width = char_width(ch, display);
            if display >= column || column < display + width {
                return start + offset;
            }
            display += width;
        }
        start + (end - start)
    }

    pub(super) fn move_to_display_position(&mut self, line: usize, column: usize) {
        let line = line.min(self.line_count().saturating_sub(1));
        self.cursor = self.byte_at_display_column(line, column);
        self.anchor = None;
        self.preferred_column = None;
        self.break_undo_group();
    }

    pub(super) fn take_dirty_from(&self) -> Option<usize> {
        self.dirty_from.take()
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(super) fn mark_saved(&mut self) {
        self.saved_hash = content_hash(&self.text);
        self.dirty = false;
        self.break_undo_group();
    }

    pub(super) fn break_undo_group(&mut self) {
        self.last_edit = None;
    }

    fn rebuild_lines(&mut self) {
        self.line_starts.clear();
        self.line_starts.push(0);
        for index in memchr::memchr_iter(b'\n', self.text.as_bytes()) {
            self.line_starts.push(index + 1);
        }
    }

    fn note_change(&mut self, line: usize) {
        self.version = self.version.wrapping_add(1);
        self.dirty = true;
        let previous = self.dirty_from.get();
        self.dirty_from
            .set(Some(previous.map_or(line, |value| value.min(line))));
        self.rebuild_lines();
        self.preferred_column = None;
    }

    fn replace_range(&mut self, start: usize, end: usize, inserted: &str) -> usize {
        let start = start.min(self.text.len());
        let end = end.min(self.text.len()).max(start);
        let removed = self.text[start..end].to_owned();
        if removed.is_empty() && inserted.is_empty() {
            return start;
        }
        let cursor_before = self.cursor;
        let anchor_before = self.anchor;
        let line = self.line_of(start);
        self.text.replace_range(start..end, inserted);
        self.cursor = start + inserted.len();
        self.anchor = None;
        self.note_change(line);
        let change = Change {
            start,
            removed,
            inserted: inserted.to_owned(),
        };
        self.push_group(change, cursor_before, anchor_before);
        self.redo.clear();
        self.last_edit = None;
        self.cursor
    }

    fn push_group(&mut self, change: Change, cursor_before: usize, anchor_before: Option<usize>) {
        self.undo.push(EditGroup {
            changes: vec![change],
            cursor_before,
            anchor_before,
            cursor_after: self.cursor,
            anchor_after: self.anchor,
        });
        if self.undo.len() > MAX_UNDO_GROUPS {
            self.undo.remove(0);
        }
    }

    fn coalesce_target(&self, kind: EditKind, now: Instant, start: usize, removed: usize) -> bool {
        let Some((at, last_kind)) = self.last_edit else {
            return false;
        };
        if !matches!(
            kind,
            EditKind::Typing | EditKind::Backspace | EditKind::Delete
        ) {
            return false;
        }
        if kind != last_kind || now.saturating_duration_since(at) > UNDO_COALESCE {
            return false;
        }
        let Some(group) = self.undo.last() else {
            return false;
        };
        let Some(change) = group.changes.last() else {
            return false;
        };
        match kind {
            EditKind::Typing => {
                change.removed.is_empty()
                    && start == change.start + change.inserted.len()
                    && !change.inserted.is_empty()
            }
            EditKind::Backspace => change.inserted.is_empty() && start + removed == change.start,
            EditKind::Delete => change.inserted.is_empty() && start == change.start,
            _ => false,
        }
    }

    fn merge_change(&mut self, kind: EditKind, change: Change) {
        let Some(group) = self.undo.last_mut() else {
            return;
        };
        let Some(last) = group.changes.last_mut() else {
            return;
        };
        match kind {
            EditKind::Typing => last.inserted.push_str(&change.inserted),
            EditKind::Backspace => {
                last.start = change.start;
                let mut removed = change.removed;
                removed.push_str(&last.removed);
                last.removed = removed;
            }
            EditKind::Delete => last.removed.push_str(&change.removed),
            _ => {}
        }
        group.cursor_after = self.cursor;
        group.anchor_after = self.anchor;
    }

    pub(super) fn insert_text(&mut self, inserted: &str) {
        self.insert_text_kind(inserted, EditKind::Paste);
    }

    pub(super) fn insert_typed(&mut self, inserted: &str) {
        self.insert_text_kind(inserted, EditKind::Typing);
    }

    fn insert_text_kind(&mut self, inserted: &str, kind: EditKind) {
        if inserted.is_empty() {
            return;
        }
        let now = Instant::now();
        let start = self.selection().map_or(self.cursor, |(start, _)| start);
        let end = self.selection().map_or(self.cursor, |(_, end)| end);
        let coalesce = self.selection().is_none()
            && self.coalesce_target(kind, now, start, end - start)
            && matches!(kind, EditKind::Typing);
        let cursor_before = self.cursor;
        let anchor_before = self.anchor;
        let line = self.line_of(start);
        let removed = self.text[start..end].to_owned();
        if removed.is_empty() && inserted.is_empty() {
            return;
        }
        self.text.replace_range(start..end, inserted);
        self.cursor = start + inserted.len();
        self.anchor = None;
        self.note_change(line);
        let change = Change {
            start,
            removed,
            inserted: inserted.to_owned(),
        };
        if coalesce {
            self.merge_change(kind, change);
        } else {
            self.push_group(change, cursor_before, anchor_before);
        }
        self.redo.clear();
        self.last_edit = Some((now, kind));
        self.preferred_column = None;
    }

    pub(super) fn insert_pair(&mut self, open: char, close: char) {
        let text = format!("{open}{close}");
        self.insert_text_kind(&text, EditKind::Typing);
        self.cursor = self.cursor.saturating_sub(close.len_utf8());
        if let Some(group) = self.undo.last_mut() {
            group.cursor_after = self.cursor;
        }
        self.preferred_column = None;
    }

    pub(super) fn wrap_selection(&mut self, open: &str, close: &str) {
        let Some((start, end)) = self.selection() else {
            return;
        };
        let inner = self.text[start..end].to_owned();
        let inserted = format!("{open}{inner}{close}");
        let cursor_before = self.cursor;
        let anchor_before = self.anchor;
        let line = self.line_of(start);
        self.text.replace_range(start..end, &inserted);
        self.cursor = start + open.len() + inner.len();
        self.anchor = Some(start + open.len());
        self.note_change(line);
        self.push_group(
            Change {
                start,
                removed: inner,
                inserted,
            },
            cursor_before,
            anchor_before,
        );
        self.redo.clear();
        self.last_edit = None;
        self.preferred_column = None;
    }

    pub(super) fn delete_pair_before(&mut self) {
        if self.cursor == 0 || self.cursor >= self.text.len() {
            self.backspace();
            return;
        }
        let previous = self.previous_byte(self.cursor);
        let next = self.next_byte(self.cursor);
        let open = self.text[previous..self.cursor].chars().next();
        let close = self.text[self.cursor..next].chars().next();
        if let (Some(open), Some(close)) = (open, close)
            && super::pair_close(open) == Some(close)
        {
            self.replace_range(previous, next, "");
            return;
        }
        self.backspace();
    }

    pub(super) fn backspace(&mut self) {
        let now = Instant::now();
        if let Some((start, end)) = self.selection() {
            self.replace_range(start, end, "");
            self.last_edit = None;
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let previous = self.previous_byte(self.cursor);
        let coalesce =
            self.coalesce_target(EditKind::Backspace, now, previous, self.cursor - previous);
        let cursor_before = self.cursor;
        let anchor_before = self.anchor;
        let line = self.line_of(previous);
        let removed = self.text[previous..self.cursor].to_owned();
        self.text.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        self.note_change(line);
        let change = Change {
            start: previous,
            removed,
            inserted: String::new(),
        };
        if coalesce {
            self.merge_change(EditKind::Backspace, change);
        } else {
            self.push_group(change, cursor_before, anchor_before);
        }
        self.redo.clear();
        self.last_edit = Some((now, EditKind::Backspace));
        self.preferred_column = None;
    }

    pub(super) fn delete(&mut self) {
        let now = Instant::now();
        if let Some((start, end)) = self.selection() {
            self.replace_range(start, end, "");
            self.last_edit = None;
            return;
        }
        if self.cursor >= self.text.len() {
            return;
        }
        let next = self.next_byte(self.cursor);
        let coalesce = self.coalesce_target(EditKind::Delete, now, self.cursor, next - self.cursor);
        let cursor_before = self.cursor;
        let anchor_before = self.anchor;
        let line = self.line_of(self.cursor);
        let removed = self.text[self.cursor..next].to_owned();
        self.text.replace_range(self.cursor..next, "");
        self.note_change(line);
        let change = Change {
            start: self.cursor,
            removed,
            inserted: String::new(),
        };
        if coalesce {
            self.merge_change(EditKind::Delete, change);
        } else {
            self.push_group(change, cursor_before, anchor_before);
        }
        self.redo.clear();
        self.last_edit = Some((now, EditKind::Delete));
        self.preferred_column = None;
    }

    pub(super) fn undo(&mut self) -> bool {
        let Some(group) = self.undo.pop() else {
            return false;
        };
        let mut first_line = usize::MAX;
        for change in group.changes.iter().rev() {
            first_line = first_line.min(self.line_of(change.start));
            let end = change.start + change.inserted.len();
            self.text.replace_range(change.start..end, &change.removed);
        }
        self.cursor = group.cursor_before;
        self.anchor = group.anchor_before;
        self.rebuild_lines();
        self.version = self.version.wrapping_add(1);
        self.dirty = content_hash(&self.text) != self.saved_hash;
        self.dirty_from.set(Some(
            self.dirty_from
                .get()
                .map_or(first_line, |v| v.min(first_line)),
        ));
        self.redo.push(group);
        self.last_edit = None;
        self.preferred_column = None;
        true
    }

    pub(super) fn redo(&mut self) -> bool {
        let Some(group) = self.redo.pop() else {
            return false;
        };
        let mut first_line = usize::MAX;
        for change in group.changes.iter() {
            first_line = first_line.min(self.line_of(change.start));
            let end = change.start + change.removed.len();
            self.text.replace_range(change.start..end, &change.inserted);
        }
        self.cursor = group.cursor_after;
        self.anchor = group.anchor_after;
        self.rebuild_lines();
        self.version = self.version.wrapping_add(1);
        self.dirty = content_hash(&self.text) != self.saved_hash;
        self.dirty_from.set(Some(
            self.dirty_from
                .get()
                .map_or(first_line, |v| v.min(first_line)),
        ));
        self.undo.push(group);
        self.last_edit = None;
        self.preferred_column = None;
        true
    }

    fn previous_byte(&self, from: usize) -> usize {
        if from >= 2 && self.text.as_bytes().get(from - 2..from) == Some(b"\r\n".as_slice()) {
            return from - 2;
        }
        self.text[..from]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_byte(&self, from: usize) -> usize {
        if self.text[from..].starts_with("\r\n") {
            return from + 2;
        }
        self.text[from..]
            .chars()
            .next()
            .map_or(from, |ch| from + ch.len_utf8())
    }

    fn set_cursor_extend(&mut self, cursor: usize, extend: bool) {
        let cursor = cursor.min(self.text.len());
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
            self.cursor = cursor;
            if self.anchor == Some(self.cursor) {
                self.anchor = None;
            }
        } else {
            self.cursor = cursor;
            self.anchor = None;
        }
        self.preferred_column = None;
        self.break_undo_group();
    }

    pub(super) fn left(&mut self, extend: bool) {
        if !extend && let Some((start, _)) = self.selection() {
            self.set_cursor_extend(start, false);
            return;
        }
        let cursor = self.previous_byte(self.cursor);
        self.set_cursor_extend(cursor, extend);
    }

    pub(super) fn right(&mut self, extend: bool) {
        if !extend && let Some((_, end)) = self.selection() {
            self.set_cursor_extend(end, false);
            return;
        }
        let cursor = self.next_byte(self.cursor);
        self.set_cursor_extend(cursor, extend);
    }

    pub(super) fn up(&mut self, extend: bool) {
        self.vertical(-1, extend);
    }

    pub(super) fn down(&mut self, extend: bool) {
        self.vertical(1, extend);
    }

    fn vertical(&mut self, delta: isize, extend: bool) {
        let line = self.line_of(self.cursor);
        let column = match self.preferred_column {
            Some(column) => column,
            None => {
                let column = self.display_column(self.cursor);
                self.preferred_column = Some(column);
                column
            }
        };
        let target = line.saturating_add_signed(delta);
        let target = target.min(self.line_count().saturating_sub(1));
        let byte = self.byte_at_display_column(target, column);
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
            self.cursor = byte;
        } else {
            self.cursor = byte;
            self.anchor = None;
        }
        self.break_undo_group();
    }

    pub(super) fn home(&mut self, extend: bool, smart: bool) {
        let line = self.line_of(self.cursor);
        let start = self.line_start(line);
        let end = self.line_end(line);
        let first_non_space = start
            + self.text[start..end]
                .char_indices()
                .find(|(_, ch)| !ch.is_whitespace())
                .map_or(end - start, |(index, _)| index);
        let target = if smart && self.cursor != first_non_space && first_non_space != start {
            first_non_space
        } else {
            start
        };
        self.set_cursor_extend(target, extend);
    }

    pub(super) fn end(&mut self, extend: bool) {
        let line = self.line_of(self.cursor);
        let target = self.line_end(line);
        self.set_cursor_extend(target, extend);
    }

    pub(super) fn doc_start(&mut self, extend: bool) {
        self.set_cursor_extend(0, extend);
    }

    pub(super) fn doc_end(&mut self, extend: bool) {
        let end = self.text.len();
        self.set_cursor_extend(end, extend);
    }

    pub(super) fn page(&mut self, delta: isize, rows: usize, extend: bool) {
        let line = self.line_of(self.cursor);
        let column = match self.preferred_column {
            Some(column) => column,
            None => {
                let column = self.display_column(self.cursor);
                self.preferred_column = Some(column);
                column
            }
        };
        let rows = rows.max(1);
        let target = line.saturating_add_signed(delta.saturating_mul(rows as isize));
        let target = target.min(self.line_count().saturating_sub(1));
        let byte = self.byte_at_display_column(target, column);
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
            self.cursor = byte;
        } else {
            self.cursor = byte;
            self.anchor = None;
        }
        self.break_undo_group();
    }

    fn word_class(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    pub(super) fn word_left(&mut self, extend: bool) {
        let target = self.word_left_byte(self.cursor);
        self.set_cursor_extend(target, extend);
    }

    pub(super) fn word_right(&mut self, extend: bool) {
        let target = self.word_right_byte(self.cursor);
        self.set_cursor_extend(target, extend);
    }

    fn word_left_byte(&self, from: usize) -> usize {
        let mut cursor = from;
        while cursor > 0 {
            let previous = self.previous_byte(cursor);
            let ch = self.text[previous..cursor].chars().next().unwrap_or(' ');
            if ch.is_whitespace() || ch == '\r' || ch == '\n' {
                cursor = previous;
            } else {
                break;
            }
        }
        if cursor == 0 {
            return 0;
        }
        let previous = self.previous_byte(cursor);
        let class = self.text[previous..cursor]
            .chars()
            .next()
            .is_some_and(Self::word_class);
        let mut start = cursor;
        while start > 0 {
            let previous = self.previous_byte(start);
            let ch = self.text[previous..start].chars().next().unwrap_or(' ');
            let same = if class {
                Self::word_class(ch)
            } else {
                !ch.is_whitespace() && !ch.is_alphanumeric() && ch != '_'
            };
            if !same {
                break;
            }
            start = previous;
        }
        start
    }

    fn word_right_byte(&self, from: usize) -> usize {
        let mut cursor = from;
        while cursor < self.text.len() {
            let next = self.next_byte(cursor);
            let ch = self.text[cursor..next].chars().next().unwrap_or(' ');
            if ch.is_whitespace() || ch == '\r' || ch == '\n' {
                cursor = next;
            } else {
                break;
            }
        }
        if cursor >= self.text.len() {
            return self.text.len();
        }
        let ch = self.text[cursor..].chars().next().unwrap_or(' ');
        let class = Self::word_class(ch);
        while cursor < self.text.len() {
            let next = self.next_byte(cursor);
            let ch = self.text[cursor..next].chars().next().unwrap_or(' ');
            let same = if class {
                Self::word_class(ch)
            } else {
                !ch.is_whitespace() && !ch.is_alphanumeric() && ch != '_'
            };
            if !same {
                break;
            }
            cursor = next;
        }
        cursor
    }

    pub(super) fn delete_word_left(&mut self) {
        let start = self.word_left_byte(self.cursor);
        let end = self.cursor;
        if start < end {
            self.replace_range(start, end, "");
        }
    }

    pub(super) fn delete_word_right(&mut self) {
        let end = self.word_right_byte(self.cursor);
        let start = self.cursor;
        if start < end {
            self.replace_range(start, end, "");
        }
    }

    fn line_indent(&self, line: usize) -> String {
        let text = self.line_text(line);
        text.chars()
            .take_while(|ch| *ch == ' ' || *ch == '\t')
            .collect()
    }

    fn indent_unit(&self) -> &'static str {
        self.indent.unit()
    }

    fn extra_indent(&self) -> bool {
        let line = self.line_of(self.cursor);
        let start = self.line_start(line);
        let before = self.text[start..self.cursor].trim_end();
        if before.ends_with('{') || before.ends_with('[') || before.ends_with('(') {
            return true;
        }
        self.python && before.ends_with(':')
    }

    pub(super) fn newline(&mut self) {
        let line = self.line_of(self.cursor);
        let indent = self.line_indent(line);
        let extra = if self.extra_indent() {
            self.indent_unit()
        } else {
            ""
        };
        let text = format!("{}{indent}{extra}", self.newline);
        self.insert_text_kind(&text, EditKind::Other);
        self.last_edit = None;
    }

    pub(super) fn tab(&mut self) {
        if self.selection().is_some() {
            self.indent_lines();
            return;
        }
        if self.indent.tabs {
            self.insert_text_kind("\t", EditKind::Typing);
            return;
        }
        let column = self.display_column(self.cursor);
        let width = self.indent.width.max(1);
        let spaces = width - column % width;
        self.insert_text_kind(&" ".repeat(spaces), EditKind::Typing);
    }

    pub(super) fn outdent(&mut self) {
        if self.selection().is_some() {
            self.outdent_lines();
        } else {
            self.outdent_current_line();
        }
    }

    fn selection_lines(&self) -> Option<(usize, usize)> {
        let (start, end) = self.selection()?;
        let first = self.line_of(start);
        let mut last = self.line_of(end);
        if last > first && end == self.line_start(last) {
            last -= 1;
        }
        Some((first, last))
    }

    pub(super) fn indent_lines(&mut self) {
        let Some((first, last)) = self.selection_lines() else {
            return;
        };
        let unit = self.indent_unit().to_owned();
        let start = self.line_start(first);
        let end = self.line_end_full(last);
        let old = self.text[start..end].to_owned();
        let (old_cursor, old_anchor) = (self.cursor, self.anchor);
        let mut new = String::with_capacity(old.len() + unit.len() * (last - first + 1));
        let mut cursor_shift = 0;
        let mut anchor_shift = 0;
        for line in first..=last {
            let line_start = self.line_start(line);
            let line_end = self.line_end_full(line);
            if line_start < old_cursor {
                cursor_shift += unit.len();
            }
            if old_anchor.is_some_and(|anchor| line_start < anchor) {
                anchor_shift += unit.len();
            }
            new.push_str(&unit);
            new.push_str(&self.text[line_start..line_end]);
        }
        let cursor = old_cursor + cursor_shift;
        let anchor = old_anchor.map(|anchor| anchor + anchor_shift);
        self.bulk_replace(start, end, &old, new, cursor, anchor, first);
    }

    pub(super) fn outdent_lines(&mut self) {
        let Some((first, last)) = self.selection_lines() else {
            return;
        };
        let width = if self.indent.tabs {
            1
        } else {
            self.indent.width
        };
        let start = self.line_start(first);
        let end = self.line_end_full(last);
        let old = self.text[start..end].to_owned();
        let (old_cursor, old_anchor) = (self.cursor, self.anchor);
        let mut new = String::with_capacity(old.len());
        let mut cursor_shift = 0;
        let mut anchor_shift = 0;
        for line in first..=last {
            let line_start = self.line_start(line);
            let line_end = self.line_end_full(line);
            let text = &self.text[line_start..line_end];
            let mut removed = 0;
            for ch in text.chars().take(width) {
                if ch == ' ' || ch == '\t' {
                    removed += ch.len_utf8();
                } else {
                    break;
                }
            }
            if removed > 0 {
                if line_start < old_cursor {
                    cursor_shift += removed.min(old_cursor - line_start);
                }
                if let Some(anchor) = old_anchor
                    && line_start < anchor
                {
                    anchor_shift += removed.min(anchor - line_start);
                }
            }
            new.push_str(&text[removed..]);
        }
        let cursor = old_cursor.saturating_sub(cursor_shift);
        let anchor = old_anchor.map(|anchor| anchor.saturating_sub(anchor_shift));
        self.bulk_replace(start, end, &old, new, cursor, anchor, first);
    }

    fn bulk_replace(
        &mut self,
        start: usize,
        end: usize,
        old: &str,
        new: String,
        cursor: usize,
        anchor: Option<usize>,
        first_line: usize,
    ) {
        let cursor_before = self.cursor;
        let anchor_before = self.anchor;
        self.text.replace_range(start..end, &new);
        self.cursor = cursor.min(self.text.len());
        self.anchor = anchor.map(|value| value.min(self.text.len()));
        self.note_change(first_line);
        self.push_group(
            Change {
                start,
                removed: old.to_owned(),
                inserted: new,
            },
            cursor_before,
            anchor_before,
        );
        self.redo.clear();
        self.last_edit = None;
    }

    fn outdent_current_line(&mut self) {
        let line = self.line_of(self.cursor);
        let start = self.line_start(line);
        let end = self.line_end(line);
        let text = &self.text[start..end];
        let mut removed = 0;
        for ch in text.chars().take(if self.indent.tabs {
            1
        } else {
            self.indent.width
        }) {
            if ch == ' ' || ch == '\t' {
                removed += 1;
            } else {
                break;
            }
        }
        if removed > 0 {
            self.replace_range(start, start + removed, "");
        }
    }

    pub(super) fn move_lines(&mut self, delta: isize) -> bool {
        if delta == 0 {
            return false;
        }
        let line = self.line_of(self.cursor);
        let target = line.saturating_add_signed(delta);
        if target >= self.line_count() {
            return false;
        }
        let current_start = self.line_start(line);
        let current_end = self.line_end_full(line);
        let target_start = self.line_start(target);
        let target_end = self.line_end_full(target);
        let cursor_in_line = self.cursor - current_start;
        let current = self.text[current_start..current_end].to_owned();
        let other = self.text[target_start..target_end].to_owned();
        let (start, end, new, cursor) = if delta < 0 {
            (
                target_start,
                current_end,
                format!("{current}{other}"),
                target_start + cursor_in_line,
            )
        } else {
            (
                current_start,
                target_end,
                format!("{other}{current}"),
                current_start + other.len() + cursor_in_line,
            )
        };
        let old = self.text[start..end].to_owned();
        self.bulk_replace(start, end, &old, new, cursor, None, line.min(target));
        true
    }

    pub(super) fn word_at(&self, byte: usize) -> Option<(usize, usize)> {
        let line = self.line_of(byte);
        let start = self.line_start(line);
        let end = self.line_end(line);
        if start == end {
            return Some((start, end));
        }
        let byte = byte.min(end.saturating_sub(1)).max(start);
        let mut begin = byte;
        while begin > start {
            let previous = self.previous_byte(begin);
            let ch = self.text[previous..begin].chars().next().unwrap_or(' ');
            if !Self::word_class(ch) {
                break;
            }
            begin = previous;
        }
        let mut finish = byte;
        while finish < end {
            let next = self.next_byte(finish);
            let ch = self.text[finish..next].chars().next().unwrap_or(' ');
            if !Self::word_class(ch) {
                break;
            }
            finish = next;
        }
        Some((begin, finish))
    }

    pub(super) fn line_bounds_at(&self, byte: usize) -> (usize, usize) {
        let line = self.line_of(byte);
        (self.line_start(line), self.line_end_full(line))
    }

    pub(super) fn matching_bracket(&self) -> Option<(usize, usize)> {
        let bytes = self.text.as_bytes();
        let at = |index: usize| bytes.get(index).copied();
        let (index, open, close, forward) = if self.cursor < self.text.len()
            && matches!(at(self.cursor), Some(b'(' | b'[' | b'{'))
        {
            (
                self.cursor,
                bytes[self.cursor],
                matching_close(bytes[self.cursor]),
                true,
            )
        } else if self.cursor < self.text.len()
            && matches!(at(self.cursor), Some(b')' | b']' | b'}'))
        {
            (
                self.cursor,
                matching_open(bytes[self.cursor]),
                bytes[self.cursor],
                false,
            )
        } else if self.cursor > 0 && matches!(at(self.cursor - 1), Some(b'(' | b'[' | b'{')) {
            (
                self.cursor - 1,
                bytes[self.cursor - 1],
                matching_close(bytes[self.cursor - 1]),
                true,
            )
        } else if self.cursor > 0 && matches!(at(self.cursor - 1), Some(b')' | b']' | b'}')) {
            (
                self.cursor - 1,
                matching_open(bytes[self.cursor - 1]),
                bytes[self.cursor - 1],
                false,
            )
        } else {
            return None;
        };
        let mut depth = 0isize;
        let mut position = index;
        let mut scanned = 0;
        loop {
            if scanned > MAX_BRACKET_SCAN {
                return None;
            }
            let byte = at(position)?;
            if byte == open {
                depth += 1;
            } else if byte == close {
                depth -= 1;
            }
            if depth == 0 {
                return Some(if forward {
                    (index, position)
                } else {
                    (position, index)
                });
            }
            if forward {
                position = position.checked_add(1)?;
                if position >= bytes.len() {
                    return None;
                }
            } else {
                if position == 0 {
                    return None;
                }
                position -= 1;
            }
            scanned += 1;
        }
    }
}

fn matching_close(open: u8) -> u8 {
    match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        other => other,
    }
}

fn matching_open(close: u8) -> u8 {
    match close {
        b')' => b'(',
        b']' => b'[',
        b'}' => b'{',
        other => other,
    }
}

fn char_width(ch: char, column: usize) -> usize {
    if ch == '\t' {
        TAB_WIDTH - column % TAB_WIDTH
    } else {
        ch.width().unwrap_or(0)
    }
}

pub(super) fn display_width(text: &str) -> usize {
    let mut column = 0;
    for ch in text.chars() {
        column += char_width(ch, column);
    }
    column
}

fn content_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn detect_indent(text: &str) -> Indent {
    let mut tabs = 0usize;
    let mut spaces = [0usize; 9];
    for line in text.lines().take(500) {
        if line.starts_with('\t') {
            tabs += 1;
        } else {
            let count = line.len() - line.trim_start_matches(' ').len();
            if count > 0 {
                spaces[count.min(8)] += 1;
            }
        }
    }
    let space_total: usize = spaces.iter().sum();
    if tabs > space_total {
        return Indent {
            tabs: true,
            width: 4,
        };
    }
    let mut width = 4;
    let mut best = 0;
    for (index, count) in spaces.iter().enumerate().skip(1) {
        if *count > best {
            best = *count;
            width = index;
        }
    }
    Indent { tabs: false, width }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::new(text.to_owned(), false)
    }

    #[test]
    fn line_index_tracks_newlines() {
        let mut document = doc("one\ntwo\r\nthree");
        assert_eq!(document.line_count(), 3);
        assert_eq!(document.line_text(1), "two");
        assert_eq!(document.line_of(4), 1);
        document.doc_end(false);
        document.insert_text("\nfour");
        assert_eq!(document.line_count(), 4);
        assert_eq!(document.line_text(3), "four");
        assert_eq!(document.row_col(document.cursor()), (3, 4));
    }

    #[test]
    fn display_columns_match_tabs_and_wide_characters() {
        let mut document = doc("甲\tb\nx");
        assert_eq!(document.display_column(0), 0);
        assert_eq!(document.display_column("甲".len()), 2);
        assert_eq!(document.display_column("甲\t".len()), 4);
        assert_eq!(document.byte_at_display_column(0, 2), "甲".len());
        assert_eq!(document.byte_at_display_column(0, 4), "甲\t".len());
        document.move_to_display_position(1, 1);
        assert_eq!(document.cursor(), document.line_end(1));
    }

    #[test]
    fn vertical_movement_keeps_display_column() {
        let mut document = doc("abcd\na\nabcdef");
        document.end(false);
        document.down(false);
        assert_eq!(document.position(), (1, 1));
        document.down(false);
        assert_eq!(document.position(), (2, 4));
    }

    #[test]
    fn selection_replacement_and_undo_redo() {
        let mut document = doc("hello world");
        document.select(0, 5);
        assert_eq!(document.selected_text(), Some("hello"));
        document.insert_typed("goodbye");
        assert_eq!(document.text(), "goodbye world");
        assert!(document.undo());
        assert_eq!(document.text(), "hello world");
        assert_eq!(document.selected_text(), Some("hello"));
        assert!(document.redo());
        assert_eq!(document.text(), "goodbye world");
    }

    #[test]
    fn typing_is_grouped_until_a_movement() {
        let mut document = doc("");
        for ch in "abc".chars() {
            document.insert_typed(&ch.to_string());
        }
        document.left(false);
        document.insert_typed("x");
        assert!(document.undo());
        assert_eq!(document.text(), "abc");
        assert!(document.undo());
        assert_eq!(document.text(), "");
    }

    #[test]
    fn backspace_and_delete_coalesce() {
        let mut document = doc("abcd");
        document.doc_end(false);
        document.backspace();
        document.backspace();
        assert!(document.undo());
        assert_eq!(document.text(), "abcd");
        document.doc_start(false);
        document.delete();
        document.delete();
        assert!(document.undo());
        assert_eq!(document.text(), "abcd");
    }

    #[test]
    fn auto_indent_and_tab_stops() {
        let mut document = doc("fn main() {");
        document.doc_end(false);
        document.newline();
        assert_eq!(document.text(), "fn main() {\n    ");
        document.outdent();
        assert_eq!(document.text(), "fn main() {\n");
    }

    #[test]
    fn word_movement_and_deletion() {
        let mut document = doc("alpha beta_gamma delta");
        document.word_right(false);
        assert_eq!(document.cursor(), 5);
        document.word_right(false);
        assert_eq!(document.cursor(), 16);
        document.delete_word_left();
        assert_eq!(document.text(), "alpha  delta");
    }

    #[test]
    fn bracket_matching_finds_pairs() {
        let mut document = doc("fn f() { g(1); }");
        document.set_cursor(5);
        let (open, close) = document.matching_bracket().unwrap();
        assert_eq!(document.text().as_bytes()[open], b'(');
        assert_eq!(document.text().as_bytes()[close], b')');
        document.set_cursor(6);
        let (open, close) = document.matching_bracket().unwrap();
        assert_eq!(document.text().as_bytes()[open], b'(');
        assert_eq!(document.text().as_bytes()[close], b')');
    }

    #[test]
    fn indent_and_outdent_lines() {
        let mut document = doc("a\nb\nc");
        document.select(0, 3);
        document.indent_lines();
        assert_eq!(document.text(), "    a\n    b\nc");
        document.outdent_lines();
        assert_eq!(document.text(), "a\nb\nc");
    }

    #[test]
    fn dirty_clears_when_undoing_to_save_point() {
        let mut document = doc("x");
        document.insert_typed("y");
        assert!(document.is_dirty());
        document.mark_saved();
        assert!(!document.is_dirty());
        document.insert_typed("z");
        assert!(document.is_dirty());
        assert!(document.undo());
        assert!(!document.is_dirty());
    }
}
