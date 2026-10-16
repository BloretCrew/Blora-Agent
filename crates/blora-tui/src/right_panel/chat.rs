// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[cfg(test)]
use ratatui::text::Line;

impl SideChat {
    pub(crate) fn title(&self) -> String {
        self.projection
            .session
            .as_ref()
            .and_then(|session| session.title.as_deref())
            .filter(|title| !title.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| i18n::tr("right.chat", "侧聊"))
    }
    pub(crate) fn permission(
        &self,
        fallback: blora_policy::PermissionMode,
    ) -> blora_policy::PermissionMode {
        self.permission.unwrap_or(fallback)
    }
    pub(crate) fn route(&self) -> Option<(&str, &str)> {
        self.route
            .as_ref()
            .map(|(model, provider)| (model.as_str(), provider.as_str()))
    }
    pub(crate) fn set_route(&mut self, model: String, provider: String) {
        self.route = Some((model, provider));
    }

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
        crate::conversation::insert_input(&mut self.draft, &mut self.cursor, text);
        self.history_index = None;
    }
    pub(super) fn edit(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Backspace
            | KeyCode::Delete => {
                crate::conversation::edit_input(&mut self.draft, &mut self.cursor, key);
                if matches!(key.code, KeyCode::Backspace | KeyCode::Delete) {
                    self.history_index = None;
                }
            }
            KeyCode::Up | KeyCode::Down => {
                crate::conversation::browse_input_history(
                    &self.history,
                    &mut self.history_index,
                    &mut self.saved_draft,
                    &mut self.draft,
                    key.code == KeyCode::Up,
                );
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
    #[cfg(test)]
    pub(super) fn lines(&self, width: usize, theme: &Theme) -> Vec<Line<'static>> {
        let mut cache = crate::view::TranscriptCache::default();
        cache.ensure(
            &self.projection,
            &self.id.to_string(),
            None,
            false,
            true,
            width,
            "你",
            theme,
        );
        cache.lines
    }
}
pub(super) use crate::conversation::draft_lines;
