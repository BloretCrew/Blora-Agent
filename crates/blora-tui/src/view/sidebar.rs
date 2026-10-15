// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::HashSet;

use super::*;

#[derive(Clone, Debug, Default)]
pub struct SidebarState {
    pub sessions: usize,
    pub left_collapsed: bool,
    pub right_collapsed: bool,
    expanded: HashSet<String>,
    active: Option<String>,
    workspace: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TreeRow {
    Project(usize, String),
    Session(usize),
}

pub(super) fn canvas_regions(
    area: Rect,
    state: &SidebarState,
) -> (Option<Rect>, Rect, Option<Rect>) {
    if area.height < 18 || area.width < 100 {
        return (None, area, None);
    }
    let left = !state.left_collapsed && area.width >= 100;
    let right = !state.right_collapsed && area.width >= 72 + if left { 52 } else { 24 };
    let left_width = if left { 28 } else { 0 };
    let right_width = if right { 24 } else { 0 };
    (
        left.then(|| Rect::new(area.x, area.y, 28, area.height)),
        Rect::new(
            area.x + left_width,
            area.y,
            area.width - left_width - right_width,
            area.height,
        ),
        right.then(|| Rect::new(area.right() - 24, area.y, 24, area.height)),
    )
}

fn tree_height(height: u16) -> usize {
    height.saturating_sub(11) as usize
}

impl SidebarState {
    fn rows(&self, sessions: &[SessionSummary]) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        for (project, path) in crate::project_paths(sessions).into_iter().enumerate() {
            rows.push(TreeRow::Project(project, path.clone()));
            if self.expanded.contains(&path) {
                rows.extend(
                    sessions
                        .iter()
                        .enumerate()
                        .filter(|(_, session)| session.workspace_path == path)
                        .map(|(index, _)| TreeRow::Session(index)),
                );
            }
        }
        rows
    }

    pub fn toggle_project(&mut self, sessions: &[SessionSummary], index: usize) {
        if let Some(path) = crate::project_paths(sessions).get(index) {
            if !self.expanded.remove(path) {
                self.expanded.insert(path.clone());
            }
        }
    }

    pub fn sync(
        &mut self,
        sessions: &[SessionSummary],
        index: usize,
        workspace: &Path,
        height: u16,
    ) {
        let active = sessions.get(index).map(|session| session.id.to_string());
        let path = sessions
            .get(index)
            .map(|session| session.workspace_path.clone())
            .unwrap_or_else(|| workspace.display().to_string());
        let changed = self.active != active || self.workspace.as_ref() != Some(&path);
        if changed {
            self.expanded.insert(path.clone());
        }
        let rows = self.rows(sessions);
        let capacity = tree_height(height);
        if changed
            && let Some(position) = rows.iter().position(|row| *row == TreeRow::Session(index))
        {
            reveal_sidebar_row(&mut self.sessions, position, capacity);
        }
        self.sessions = self.sessions.min(rows.len().saturating_sub(capacity));
        self.active = active;
        self.workspace = Some(path);
    }

    pub fn scroll(&mut self, hits: &HitMap, col: u16, row: u16, up: bool) -> bool {
        if hits.modal {
            return false;
        }
        if hits
            .workspace_sidebar
            .is_some_and(|rect| contains(rect, col, row))
        {
            return true;
        }
        if !hits.sidebar.is_some_and(|rect| contains(rect, col, row)) {
            return false;
        }
        if hits
            .sidebar_sessions
            .is_some_and(|rect| contains(rect, col, row))
        {
            self.sessions = if up {
                self.sessions.saturating_sub(3)
            } else {
                self.sessions
                    .saturating_add(3)
                    .min(hits.sidebar_session_max)
            };
        }
        true
    }
}

fn reveal_sidebar_row(offset: &mut usize, position: usize, height: usize) {
    if position < *offset {
        *offset = position;
    } else if position >= offset.saturating_add(height) {
        *offset = position.saturating_add(1).saturating_sub(height);
    }
}
fn sidebar_row(
    frame: &mut Frame<'_>,
    area: Rect,
    text: &str,
    hit: Option<Hit>,
    selected: bool,
    model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let hovered = hit.is_some()
        && model
            .pointer
            .is_some_and(|(col, row)| contains(area, col, row));
    let style = if selected {
        theme.rose_bold().bg(theme.bg_select)
    } else if hovered {
        theme.base().bg(theme.bg_select)
    } else if hit.is_none() || matches!(hit, Some(Hit::ProjectPicker)) {
        theme.dim().bg(theme.bg_raised)
    } else {
        theme.base().bg(theme.bg_raised)
    };
    let text = ellipsize(&sanitize_title(text), area.width.saturating_sub(3) as usize);
    let marker = if selected { "›" } else { " " };
    frame.render_widget(
        Paragraph::new(format!("{marker} {text}")).style(style),
        area,
    );
    if let Some(hit) = hit {
        hits.sidebar_rows.push((area, hit));
    }
}

fn panel_background(frame: &mut Frame<'_>, area: Rect, theme: &Theme, right: bool) {
    frame.render_widget(
        Block::default().style(theme.base().bg(theme.bg_raised)),
        area,
    );
    let x = if right { area.x } else { area.right() - 1 };
    for y in area.y..area.bottom() {
        frame.render_widget(
            Paragraph::new("│").style(theme.fg(theme.hairline)),
            Rect::new(x, y, 1, 1),
        );
    }
}

pub(super) fn render_sidebar(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    hits.sidebar = Some(area);
    panel_background(frame, area, theme, false);
    let row = |y| Rect::new(area.x + 1, area.y + y, area.width - 3, 1);
    let mut paint = |y, text: &str, hit, selected| {
        sidebar_row(frame, row(y), text, hit, selected, model, theme, hits)
    };
    paint(1, "✦ Blora  ‹", Some(Hit::ToggleLeft), false);
    paint(
        3,
        &i18n::tr("sidebar.new", "新建会话"),
        Some(Hit::Hint(HintAction::New)),
        false,
    );
    paint(
        4,
        &i18n::tr("sidebar.search", "搜索会话"),
        Some(Hit::SessionPicker),
        false,
    );
    paint(
        6,
        &i18n::tr("sidebar.open_project", "打开项目"),
        Some(Hit::ProjectPicker),
        false,
    );
    let rows = model.sidebar.rows(model.sessions);
    let capacity = tree_height(area.height);
    let max = rows.len().saturating_sub(capacity);
    if rows.is_empty() {
        paint(7, &i18n::tr("sidebar.empty", "暂无会话"), None, false);
    }
    for (visible, item) in rows
        .iter()
        .skip(model.sidebar.sessions.min(max))
        .take(capacity)
        .enumerate()
    {
        match item {
            TreeRow::Project(index, path) => {
                let name = Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(path);
                let marker = if model.sidebar.expanded.contains(path) {
                    "▾"
                } else {
                    "▸"
                };
                paint(
                    7 + visible as u16,
                    &format!("{marker} / {name}"),
                    Some(Hit::SidebarProject(*index)),
                    false,
                );
            }
            TreeRow::Session(index) => {
                let title = model.sessions[*index]
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|title| !title.is_empty() && *title != "tui")
                    .map(str::to_owned)
                    .unwrap_or_else(|| i18n::tr("session.untitled", "未命名会话"));
                paint(
                    7 + visible as u16,
                    &format!("  {title}"),
                    Some(Hit::SidebarSession(*index)),
                    *index == model.index,
                );
            }
        }
    }
    paint(area.height - 3, "──────────────────────", None, false);
    paint(
        area.height - 2,
        &i18n::tr("sidebar.settings", "设置"),
        Some(Hit::SidebarSettings),
        false,
    );
    let user = if model.user_label.trim().is_empty() || model.user_label == "you" {
        i18n::tr("sidebar.guest", "未登录")
    } else {
        model.user_label.to_owned()
    };
    paint(area.height - 1, &user, None, false);
    hits.sidebar_sessions = Some(Rect::new(
        area.x,
        area.y + 7,
        area.width - 1,
        capacity as u16,
    ));
    hits.sidebar_session_max = max;
}

pub(super) fn render_workspace(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    hits.workspace_sidebar = Some(area);
    panel_background(frame, area, theme, true);
    let mut paint = |y, text: &str, hit| {
        sidebar_row(
            frame,
            Rect::new(area.x + 1, area.y + y, area.width - 2, 1),
            text,
            hit,
            false,
            model,
            theme,
            hits,
        )
    };
    paint(
        1,
        &format!("{}  ›", i18n::tr("sidebar.workspace", "工作区")),
        Some(Hit::ToggleRight),
    );
    let path = model
        .sessions
        .get(model.index)
        .map(|session| Path::new(&session.workspace_path))
        .unwrap_or(model.workspace);
    paint(
        3,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| path.to_str().unwrap_or("/")),
        None,
    );
    if let Some(git) = model.git_status {
        paint(4, &format!("⎇ {}", git.branch), Some(Hit::GitStatus));
        let summary = if git.clean {
            i18n::tr("sidebar.clean", "工作区干净")
        } else {
            format!("+{} −{}", git.added, git.removed)
        };
        paint(5, &summary, Some(Hit::GitStatus));
        paint(
            6,
            &format!("↑{} ↓{}", git.ahead, git.behind),
            Some(Hit::GitStatus),
        );
    } else {
        paint(4, &i18n::tr("sidebar.no_git", "无 Git 状态"), None);
    }
    for (y, key, fallback, hit) in [
        (8, "sidebar.changes", "变更", Hit::GitStatus),
        (10, "sidebar.files", "文件", Hit::WorkspaceFiles),
        (12, "sidebar.context", "上下文", Hit::ContextUsage),
        (14, "sidebar.tasks", "任务", Hit::WorkspaceTasks),
    ] {
        paint(y, &i18n::tr(key, fallback), Some(hit));
    }
}

pub(super) fn render_toggles(
    frame: &mut Frame<'_>,
    area: Rect,
    _model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    if area.width < 10 {
        return;
    }
    let entries = if area.width < 40 {
        [(3, "◀", Hit::ToggleLeft), (1, "▶", Hit::ToggleRight)]
    } else {
        [(10, "[◀]", Hit::ToggleLeft), (5, "[▶]", Hit::ToggleRight)]
    };
    for (offset, text, hit) in entries {
        let rect = Rect::new(area.right() - offset, area.y, text.width() as u16, 1);
        frame.render_widget(Paragraph::new(text).style(theme.dim()), rect);
        hits.sidebar_rows.push((rect, hit));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_expand_independently_and_manual_collapse_survives_sync() {
        let mut sessions = super::super::tests::picker_sessions(4);
        sessions[2].workspace_path = "/second".into();
        sessions[3].workspace_path = "/second".into();
        let mut state = SidebarState::default();
        state.sync(&sessions, 0, Path::new("/"), 18);
        assert!(state.rows(&sessions).contains(&TreeRow::Session(0)));
        assert!(!state.rows(&sessions).contains(&TreeRow::Session(2)));
        let paths = crate::project_paths(&sessions);
        let second = paths.iter().position(|path| path == "/second").unwrap();
        let first = 1 - second;
        state.toggle_project(&sessions, second);
        assert!(state.rows(&sessions).contains(&TreeRow::Session(2)));
        assert!(state.rows(&sessions).contains(&TreeRow::Session(0)));
        state.toggle_project(&sessions, first);
        state.sync(&sessions, 0, Path::new("/"), 18);
        assert!(!state.rows(&sessions).contains(&TreeRow::Session(0)));
        state.sync(&sessions, 1, Path::new("/"), 18);
        assert!(state.rows(&sessions).contains(&TreeRow::Session(1)));
        assert!(state.rows(&sessions).contains(&TreeRow::Session(2)));
        state.left_collapsed = true;
        state.right_collapsed = true;
        canvas_regions(Rect::new(0, 0, 24, 10), &state);
        state.sync(&sessions, 1, Path::new("/"), 40);
        assert!(state.left_collapsed && state.right_collapsed);
    }
}
