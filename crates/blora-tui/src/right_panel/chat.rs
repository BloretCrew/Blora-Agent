// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use ratatui::text::Line;
use unicode_width::UnicodeWidthChar;

impl SideChat {
    pub(super) fn viewport(&self, count: usize, height: usize) -> usize {
        let previous = self.rendered.replace(count);
        if self.scroll.get() > 0 && count > previous {
            self.scroll
                .set(self.scroll.get().saturating_add(count - previous));
            self.unread
                .set(self.unread.get().saturating_add(count - previous));
        }
        let max = count.saturating_sub(height);
        self.max_scroll.set(max);
        let distance = self.scroll.get().min(max);
        self.scroll.set(distance);
        if distance == 0 {
            self.unread.set(0);
        }
        distance
    }
    pub(super) fn insert(&mut self, text: &str) {
        self.draft.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.history_index = None;
    }
    pub(super) fn edit(&mut self, key: crossterm::event::KeyEvent) {
        let prev = self.draft[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        let next = self.draft[self.cursor..]
            .chars()
            .next()
            .map(|c| self.cursor + c.len_utf8())
            .unwrap_or(self.cursor);
        match key.code {
            KeyCode::Left => self.cursor = prev,
            KeyCode::Right => self.cursor = next,
            KeyCode::Home => {
                self.cursor = self.draft[..self.cursor]
                    .rfind('\n')
                    .map(|i| i + 1)
                    .unwrap_or(0)
            }
            KeyCode::End => {
                self.cursor = self.draft[self.cursor..]
                    .find('\n')
                    .map(|i| self.cursor + i)
                    .unwrap_or(self.draft.len())
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.draft.replace_range(prev..self.cursor, "");
                self.cursor = prev;
            }
            KeyCode::Delete => {
                self.draft.replace_range(self.cursor..next, "");
            }
            KeyCode::Up | KeyCode::Down => {
                if self.history.is_empty() {
                    return;
                }
                if self.history_index.is_none() {
                    self.saved_draft = self.draft.clone();
                }
                let index = self.history_index.unwrap_or(self.history.len());
                let index = if key.code == KeyCode::Up {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(self.history.len())
                };
                self.history_index = Some(index);
                self.draft = self
                    .history
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| self.saved_draft.clone());
                self.cursor = self.draft.len();
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                self.insert("\n")
            }
            KeyCode::Enter => self.send(),
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
            }
            _ => {}
        }
    }
    pub(super) fn send(&mut self) {
        if self.draft.trim().is_empty() {
            return;
        }
        self.history.push(self.draft.clone());
        if self.history.len() > 100 {
            self.history.remove(0);
        }
        self.queue.push_back(std::mem::take(&mut self.draft));
        self.cursor = 0;
        self.history_index = None;
        self.scroll.set(0);
    }
    pub(super) fn lines(&self, width: usize, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for item in &self.projection.transcript {
            match item {
                TranscriptItem::User { text, .. } => {
                    lines.push(Line::styled(
                        i18n::tr("right.user", "你"),
                        theme.fg(theme.rose),
                    ));
                    lines.extend(crate::markdown::render(text, width, None, theme));
                }
                TranscriptItem::Assistant { text, .. } => {
                    lines.push(Line::styled("Blora", theme.fg(theme.rose)));
                    lines.extend(crate::markdown::render(text, width, None, theme));
                }
                TranscriptItem::Tool { name, status, .. } => lines.push(Line::styled(
                    format!("[{name}: {status}]"),
                    theme.fg(theme.text_dim),
                )),
                TranscriptItem::Question { question, .. } => {
                    lines.extend(crate::markdown::render(question, width, None, theme))
                }
                _ => continue,
            }
            lines.push(Line::default());
        }
        lines
    }
}
pub(super) fn draft_lines(
    text: &str,
    cursor: usize,
    width: usize,
) -> (Vec<String>, (usize, usize)) {
    let width = width.max(1);
    let mut lines = vec![String::new()];
    let (mut row, mut col) = (0, 0);
    let mut position = (0, 0);
    for (offset, c) in text.char_indices() {
        let w = c.width().unwrap_or(0).min(width);
        if c != '\n' && col + w > width {
            lines.push(String::new());
            row += 1;
            col = 0;
        }
        if offset == cursor {
            position = (row, col);
        }
        if c == '\n' {
            lines.push(String::new());
            row += 1;
            col = 0;
        } else {
            lines[row].push(c);
            col += w;
        }
    }
    if cursor == text.len() {
        if col == width {
            lines.push(String::new());
            row += 1;
            col = 0;
        }
        position = (row, col);
    }
    (lines, position)
}
