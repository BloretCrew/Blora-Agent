// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[cfg(test)]
use ratatui::text::Line;

impl SideChat {
    pub(super) fn run_command(&mut self, runtime: &Runtime, command: &str) {
        let trimmed = command.trim();
        let (name, args) = trimmed
            .strip_prefix('/')
            .unwrap_or(trimmed)
            .split_once(' ')
            .map_or((trimmed.trim_start_matches('/'), ""), |(name, args)| {
                (name, args.trim())
            });
        let allowed = matches!(
            name,
            "id" | "status"
                | "context"
                | "compact"
                | "checkpoint"
                | "cancel"
                | "resume"
                | "archive"
                | "usage"
                | "approvals"
                | "steer"
                | "mode"
        );
        self.slash_notice = Some(if !allowed {
            format!("侧聊暂不支持 /{name}")
        } else {
            match name {
                "id" => self.id.to_string(),
                "status" => runtime
                    .show_session(&self.id)
                    .map(|p| {
                        format!(
                            "{} · seq {} · {}",
                            self.title(),
                            p.last_sequence,
                            if self.running { "running" } else { "idle" }
                        )
                    })
                    .unwrap_or_else(|e| e.to_string()),
                "context" => runtime
                    .show_session(&self.id)
                    .map(|p| {
                        format!(
                            "seq={} in={} out={} ws={}",
                            p.last_sequence,
                            p.input_tokens,
                            p.output_tokens,
                            p.session.map(|s| s.workspace_path).unwrap_or_default()
                        )
                    })
                    .unwrap_or_else(|e| e.to_string()),
                "compact" => runtime
                    .compact(&self.id)
                    .map(|_| {
                        if args.is_empty() {
                            "compacted".to_owned()
                        } else {
                            format!("compacted ({args})")
                        }
                    })
                    .unwrap_or_else(|e| e.to_string()),
                "checkpoint" => runtime
                    .checkpoint(&self.id, None, Some("manual"))
                    .map(|_| "checkpointed".to_owned())
                    .unwrap_or_else(|e| e.to_string()),
                "cancel" => {
                    self.cancel.cancel();
                    self.queue.clear();
                    "side chat cancelled; queue cleared".to_owned()
                }
                "resume" => runtime
                    .resume_session(&self.id)
                    .map(|_| "resumed".to_owned())
                    .unwrap_or_else(|e| e.to_string()),
                "archive" => runtime
                    .archive_session(&self.id)
                    .map(|_| "archived".to_owned())
                    .unwrap_or_else(|e| e.to_string()),
                "usage" => runtime
                    .usage(Some(&self.id))
                    .map(|u| {
                        format!(
                            "in={} out={} cached={}",
                            u.input_tokens, u.output_tokens, u.cached_tokens
                        )
                    })
                    .unwrap_or_else(|e| e.to_string()),
                "approvals" => runtime
                    .pending_approvals(&self.id)
                    .map(|rows| {
                        if rows.is_empty() {
                            "no pending approvals".to_owned()
                        } else {
                            rows.into_iter()
                                .map(|r| format!("{} {}", r.id, r.summary))
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    })
                    .unwrap_or_else(|e| e.to_string()),
                "steer" if !args.is_empty() => runtime
                    .queue_steer(&self.id, args)
                    .map(|_| "message queued for this side chat".to_owned())
                    .unwrap_or_else(|e| e.to_string()),
                "steer" => "usage: /steer <message>".to_owned(),
                "mode" => runtime
                    .show_session(&self.id)
                    .ok()
                    .and_then(|p| p.session)
                    .map(|s| s.mode.as_str().to_owned())
                    .unwrap_or_else(|| "unknown".to_owned()),
                _ => unreachable!("side chat command allowlist must match implementation"),
            }
        });
        self.draft.clear();
        self.cursor = 0;
        self.history_index = None;
        self.slash_selected = 0;
        self.slash_dismissed = false;
    }
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
    pub(super) fn scroll_page(&self, down: bool, page: usize) {
        let next = if down {
            self.scroll.get().saturating_sub(page)
        } else {
            self.scroll
                .get()
                .saturating_add(page)
                .min(self.max_scroll.get())
        };
        self.scroll.set(next);
        if next == 0 {
            self.unread.set(0);
        }
    }
    pub(super) fn insert(&mut self, text: &str) {
        crate::conversation::insert_input(&mut self.draft, &mut self.cursor, text);
        self.history_index = None;
        self.slash_dismissed = false;
        self.slash_notice = None;
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
                self.slash_dismissed = false;
                self.slash_notice = None;
            }
            KeyCode::Up | KeyCode::Down
                if self.history_index.is_some() || !self.draft.contains('\n') =>
            {
                crate::conversation::browse_input_history(
                    &self.history,
                    &mut self.history_index,
                    &mut self.saved_draft,
                    &mut self.draft,
                    key.code == KeyCode::Up,
                );
                self.cursor = self.draft.len();
            }
            KeyCode::Up | KeyCode::Down => {
                crate::conversation::edit_input(&mut self.draft, &mut self.cursor, key);
                self.slash_dismissed = false;
                self.slash_notice = None;
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
