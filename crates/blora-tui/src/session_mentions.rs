// SPDX-License-Identifier: GPL-3.0-or-later
use std::{cell::RefCell, ops::Range};

use blora_storage::SessionSummary;
use blora_types::SessionId;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{Frame, layout::Rect, text::Line, widgets::Paragraph};

use crate::{i18n, theme::Theme};

const MAX_VISIBLE: usize = 5;

#[derive(Default)]
pub(crate) struct SessionMentions {
    rows: Vec<(SessionId, String)>,
    range: Option<Range<usize>>,
    selected: usize,
    signature: String,
    dismissed: Option<String>,
    hits: RefCell<Vec<(Rect, usize)>>,
}

fn query(input: &str, cursor: usize) -> Option<(Range<usize>, &str)> {
    if input.starts_with('/') || !input.is_char_boundary(cursor) {
        return None;
    }
    let prefix = &input[..cursor];
    let start = prefix
        .rfind(char::is_whitespace)
        .map_or(0, |i| i + prefix[i..].chars().next().unwrap().len_utf8());
    let token = &prefix[start..];
    let needle = token.strip_prefix('@')?;
    // Complete stable tokens are ordinary message text, never completion queries.
    if needle.contains(['{', '}', '@']) {
        return None;
    }
    let end = input[cursor..]
        .find(char::is_whitespace)
        .map_or(input.len(), |i| cursor + i);
    Some((start..end, needle))
}

impl SessionMentions {
    pub(crate) fn sync(
        &mut self,
        input: &str,
        cursor: usize,
        owner: Option<&SessionId>,
        sessions: &[SessionSummary],
    ) {
        self.hits.borrow_mut().clear();
        self.range = None;
        self.rows.clear();
        let signature = format!("{owner:?}:{cursor}:{input}");
        if signature != self.signature {
            self.selected = 0;
            self.dismissed = None;
            self.signature = signature;
        }
        if self.dismissed.as_ref() == Some(&self.signature) {
            return;
        }
        let Some((range, needle)) = query(input, cursor) else {
            return;
        };
        let needle = needle.to_lowercase();
        self.range = Some(range);
        self.rows = sessions
            .iter()
            .filter(|s| Some(&s.id) != owner)
            .filter_map(|s| {
                let title = s
                    .title
                    .as_deref()
                    .filter(|title| !title.trim().is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| i18n::tr("mention.untitled", "未命名会话"));
                let label = format!("{title} · {} · {}", s.workspace_path, s.id);
                label
                    .to_lowercase()
                    .contains(&needle)
                    .then(|| (s.id.clone(), label))
            })
            .collect();
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    pub(crate) fn height(&self) -> u16 {
        if self.range.is_some() {
            1 + self.rows.len().clamp(1, MAX_VISIBLE) as u16
        } else {
            0
        }
    }

    fn choose(&mut self, index: usize, input: &mut String, cursor: &mut usize) {
        if let (Some(range), Some((id, _))) = (self.range.clone(), self.rows.get(index)) {
            let token = format!("@{{{id}}} ");
            *cursor = range.start + token.len();
            input.replace_range(range, &token);
            self.range = None;
            self.hits.borrow_mut().clear();
        }
    }

    pub(crate) fn event(&mut self, event: &Event, input: &mut String, cursor: &mut usize) -> bool {
        if self.range.is_none() {
            return false;
        }
        match event {
            Event::Key(key)
                if key.kind == KeyEventKind::Press
                    && !key.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
                    ) =>
            {
                match key.code {
                    KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                    KeyCode::Down => {
                        self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
                    }
                    KeyCode::Tab | KeyCode::Enter => self.choose(self.selected, input, cursor),
                    KeyCode::Esc => {
                        self.dismissed = Some(self.signature.clone());
                        self.range = None;
                        self.hits.borrow_mut().clear();
                    }
                    _ => return false,
                }
                true
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let index = self
                    .hits
                    .borrow()
                    .iter()
                    .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                    .map(|(_, index)| *index);
                if let Some(index) = index {
                    self.choose(index, input, cursor);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub(crate) fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.hits.borrow_mut().clear();
        if self.range.is_none() || area.height == 0 {
            return;
        }
        let header = i18n::tr(
            "mention.hint",
            "@ 会话 · ↑↓ 选择 · Tab/Enter 插入 · Esc 关闭",
        );
        frame.render_widget(
            Paragraph::new(header).style(theme.mute()),
            Rect::new(area.x, area.y, area.width, 1),
        );
        let capacity = area.height.saturating_sub(1) as usize;
        let start = self.selected.saturating_sub(capacity.saturating_sub(1));
        if self.rows.is_empty() && capacity > 0 {
            frame.render_widget(
                Paragraph::new(i18n::tr("mention.empty", "没有匹配的其他会话")).style(theme.mute()),
                Rect::new(area.x, area.y + 1, area.width, 1),
            );
        }
        for (index, (_, label)) in self.rows.iter().enumerate().skip(start).take(capacity) {
            let rect = Rect::new(area.x, area.y + 1 + (index - start) as u16, area.width, 1);
            let text = format!("{} {label}", if index == self.selected { "›" } else { " " });
            frame.render_widget(
                Paragraph::new(Line::from(text)).style(if index == self.selected {
                    theme.rose_bold()
                } else {
                    theme.base()
                }),
                rect,
            );
            self.hits.borrow_mut().push((rect, index));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, MouseEvent};
    use ratatui::{Terminal, backend::TestBackend};

    fn session(id: &str, title: &str, project: &str) -> SessionSummary {
        SessionSummary {
            id: SessionId::parse(id).unwrap(),
            title: Some(title.into()),
            workspace_path: project.into(),
            mode: blora_types::Mode::Code,
            status: blora_types::SessionStatus::Active,
            updated_at: chrono::Utc::now(),
            last_sequence: 0,
        }
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn filters_owner_title_id_and_project_and_disambiguates_duplicates() {
        let sessions = vec![
            session("ses_owner", "共同标题", "/a"),
            session("ses_first", "共同标题", "/alpha"),
            session("ses_second", "共同标题", "/beta"),
        ];
        let mut mentions = SessionMentions::default();
        for (input, expected) in [
            ("@共同", 2),
            ("@alpha", 1),
            ("@ses_second", 1),
            ("@missing", 0),
        ] {
            mentions.sync(input, input.len(), Some(&sessions[0].id), &sessions);
            assert_eq!(mentions.rows.len(), expected);
        }
        mentions.sync("@", 1, Some(&sessions[0].id), &sessions);
        assert_ne!(mentions.rows[0].1, mentions.rows[1].1);
        assert!(mentions.rows[0].1.contains("/alpha"));
        assert!(mentions.rows[1].1.contains("ses_second"));
    }

    #[test]
    fn navigation_inserts_full_token_and_preserves_unicode_and_suffix() {
        let sessions = vec![
            session("ses_first_full_id", "一", "/a"),
            session("ses_second_full_id", "二", "/b"),
        ];
        let mut mentions = SessionMentions::default();
        let mut input = "中文 @ 后文".to_owned();
        let mut cursor = "中文 @".len();
        mentions.sync(&input, cursor, None, &sessions);
        assert!(mentions.event(&key(KeyCode::Down), &mut input, &mut cursor));
        assert!(mentions.event(&key(KeyCode::Up), &mut input, &mut cursor));
        assert!(mentions.event(&key(KeyCode::Down), &mut input, &mut cursor));
        assert!(mentions.event(&key(KeyCode::Tab), &mut input, &mut cursor));
        assert_eq!(input, "中文 @{ses_second_full_id}  后文");
        assert_eq!(&input[..cursor], "中文 @{ses_second_full_id} ");
        assert_eq!(mentions.height(), 0);
    }

    #[test]
    fn escape_retains_draft_empty_results_capture_enter_and_tokens_are_literal() {
        let mut mentions = SessionMentions::default();
        let mut input = "@unknown".to_owned();
        let mut cursor = input.len();
        mentions.sync(&input, cursor, None, &[]);
        assert!(mentions.event(&key(KeyCode::Enter), &mut input, &mut cursor));
        assert_eq!(input, "@unknown");
        assert!(mentions.event(&key(KeyCode::Esc), &mut input, &mut cursor));
        mentions.sync(&input, cursor, None, &[]);
        assert_eq!(mentions.height(), 0);
        input.push('x');
        cursor += 1;
        mentions.sync(&input, cursor, None, &[]);
        assert!(mentions.height() > 0);
        for input in [
            "@{ses_complete_id}",
            "/help @",
            "mail@example.com",
            "@{ses_partial",
            "普通文本",
        ] {
            mentions.sync(input, input.len(), None, &[]);
            assert_eq!(mentions.height(), 0, "{input}");
        }
    }

    #[test]
    fn rendered_mouse_rows_insert_and_hidden_rows_cannot_be_clicked() {
        let sessions = vec![session("ses_target", "目标", "/project")];
        let mut mentions = SessionMentions::default();
        let mut input = "@".to_owned();
        let mut cursor = input.len();
        mentions.sync(&input, cursor, None, &sessions);
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal
            .draw(|frame| mentions.render(frame, Rect::new(2, 3, 60, 2), &Theme::current()))
            .unwrap();
        let click = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 4,
            modifiers: KeyModifiers::NONE,
        });
        assert!(mentions.event(&click, &mut input, &mut cursor));
        assert_eq!(input, "@{ses_target} ");
        assert!(!mentions.event(&click, &mut input, &mut cursor));
    }

    #[test]
    fn independent_owners_and_selection_do_not_leak() {
        let sessions = vec![
            session("ses_main", "主", "/a"),
            session("ses_side", "侧", "/a"),
            session("ses_other", "其他", "/a"),
        ];
        let mut main = SessionMentions::default();
        let mut side = SessionMentions::default();
        main.sync("@", 1, Some(&sessions[0].id), &sessions);
        side.sync("@", 1, Some(&sessions[1].id), &sessions);
        let mut draft = "@".to_owned();
        let mut cursor = 1;
        main.event(&key(KeyCode::Down), &mut draft, &mut cursor);
        assert_eq!(side.selected, 0);
        assert_eq!(main.rows[0].0, sessions[1].id);
        assert_eq!(side.rows[0].0, sessions[0].id);
    }
}
