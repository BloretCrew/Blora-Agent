// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Independent right-hand tools and their project/session ownership.

mod chat;
mod editor;
mod files;
mod layout;
mod terminal;
use files::*;
use terminal::terminal_style;

use crate::{i18n, theme::Theme, view::FrameModel};
use blora_runtime::{CancelToken, RunOptions, Runtime};
use blora_session::SessionProjection;
#[cfg(test)]
use blora_session::TranscriptItem;
use blora_storage::CreateSession;
use blora_types::{Mode, SessionId};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Info,
    Files,
    Terminal,
    Chat,
}
impl Tab {
    fn label(self) -> String {
        let (key, fallback) = match self {
            Self::Info => ("right.info", "信息"),
            Self::Files => ("right.files", "文件"),
            Self::Terminal => ("right.terminal", "终端"),
            Self::Chat => ("right.chat", "侧聊"),
        };
        i18n::tr(key, fallback)
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Tab(Tab),
    Close(Tab),
    Collapse,
    Restore,
    File(usize),
    Document(PathBuf),
    CloseDocument(PathBuf),
    FileSave,
    CompareKeep,
    CompareReload,
    FileConfirm(bool),
    FileCancel,
    Back,
    Explorer,
    DocumentCycle(bool),
    Focus,
    Refresh,
    Restart,
    Send,
    Cancel,
    Bottom,
    ExpandId,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ControlOwner {
    #[default]
    Main,
    Side(SessionId),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Provider,
    Permission,
    Context,
}
pub struct SideChat {
    pub id: SessionId,
    pub workspace: PathBuf,
    pub projection: SessionProjection,
    pub cancel: CancelToken,
    pub running: bool,
    draft: String,
    cursor: usize,
    mentions: crate::session_mentions::SessionMentions,
    paste_preview: Option<String>,
    history: Vec<String>,
    history_index: Option<usize>,
    saved_draft: String,
    slash_selected: usize,
    slash_notice: Option<String>,
    slash_dismissed: bool,
    rendered: std::cell::Cell<usize>,
    max_scroll: std::cell::Cell<usize>,
    unread: std::cell::Cell<usize>,
    queue: VecDeque<String>,
    scroll: std::cell::Cell<usize>,
    pub status: String,
    route: Option<(String, String)>,
    permission: Option<blora_policy::PermissionMode>,
    cache: std::cell::RefCell<crate::view::TranscriptCache>,
    tool_rows: std::cell::RefCell<Vec<(Rect, Vec<usize>)>>,
}
pub struct RunRequest {
    pub id: SessionId,
    pub prompt: String,
    pub cancel: CancelToken,
    pub options: RunOptions,
}
pub struct RightPanel {
    pub control_owner: ControlOwner,
    controls: std::cell::RefCell<Vec<(Rect, Control)>>,
    tabs: Vec<Tab>,
    active: Tab,
    pub focused: bool,
    collapsed: bool,
    preferred_width: u16,
    area: std::cell::Cell<Option<Rect>>,
    mobile_drawer: std::cell::Cell<bool>,
    hits: std::cell::RefCell<Vec<(Rect, Action)>>,
    files: Files,
    pub chat: Option<SideChat>,
    terminal: Option<crate::right_terminal::TerminalPane>,
    terminal_error: String,
    info_scroll: std::cell::Cell<usize>,
    info_max: std::cell::Cell<usize>,
    expanded_id: bool,
    chat_error: String,
    terminal_workspace: PathBuf,
    editor_area: std::cell::Cell<Option<Rect>>,
    editor_dragging: bool,
    explorer_focused: bool,
    explorer_area: std::cell::Cell<Option<Rect>>,
    pub editor_focus: bool,
    pub reduce_motion: bool,
    editor_progress: std::cell::Cell<f64>,
    editor_transition_at: std::cell::Cell<Option<std::time::Instant>>,
    editor_transition_from: std::cell::Cell<f64>,
    done_tx: Sender<(SessionId, String)>,
    done_rx: Receiver<(SessionId, String)>,
    cancellations: std::sync::Arc<std::sync::Mutex<Vec<CancelToken>>>,
}
impl Default for RightPanel {
    fn default() -> Self {
        let (done_tx, done_rx) = mpsc::channel();
        Self {
            control_owner: ControlOwner::Main,
            controls: Default::default(),
            tabs: vec![Tab::Info],
            active: Tab::Info,
            focused: false,
            collapsed: false,
            preferred_width: 36,
            area: None.into(),
            mobile_drawer: false.into(),
            hits: std::cell::RefCell::new(vec![]),
            files: Files::new(),
            chat: None,
            terminal: None,
            terminal_error: String::new(),
            info_scroll: 0.into(),
            info_max: 0.into(),
            expanded_id: false,
            chat_error: String::new(),
            terminal_workspace: PathBuf::new(),
            editor_area: std::cell::Cell::new(None),
            editor_dragging: false,
            explorer_focused: false,
            explorer_area: None.into(),
            editor_focus: false,
            reduce_motion: false,
            editor_progress: 0.0.into(),
            editor_transition_at: std::cell::Cell::new(None),
            editor_transition_from: 0.0.into(),
            done_tx,
            done_rx,
            cancellations: Default::default(),
        }
    }
}
pub fn regions(area: Rect, collapsed: bool) -> (Rect, Option<Rect>) {
    regions_with_width(area, collapsed, 36)
}
fn regions_with_width(area: Rect, collapsed: bool, preferred_width: u16) -> (Rect, Option<Rect>) {
    if !collapsed && area.width >= 124 && area.height >= 18 {
        let width = preferred_width.clamp(24, area.width.saturating_sub(88));
        (
            Rect::new(area.x, area.y, area.width - width, area.height),
            Some(Rect::new(area.right() - width, area.y, width, area.height)),
        )
    } else {
        (area, None)
    }
}
impl Drop for RightPanel {
    fn drop(&mut self) {
        if let Ok(tokens) = self.cancellations.lock() {
            for token in tokens.iter() {
                token.cancel();
            }
        }
    }
}
pub struct CancelGuard(std::sync::Arc<std::sync::Mutex<Vec<CancelToken>>>);
impl Drop for CancelGuard {
    fn drop(&mut self) {
        if let Ok(tokens) = self.0.lock() {
            for token in tokens.iter() {
                token.cancel();
            }
        }
    }
}
impl RightPanel {
    pub(crate) fn regions(&self, area: Rect) -> (Rect, Option<Rect>) {
        regions_with_width(area, self.collapsed, self.preferred_width)
    }
    pub(crate) fn resize(&mut self, terminal: Rect, column: u16) {
        self.preferred_width = terminal
            .right()
            .saturating_sub(column)
            .clamp(24, terminal.width.saturating_sub(88).max(24));
    }
    pub(crate) fn resize_hit(&self, column: u16, row: u16) -> bool {
        !self.mobile_drawer.get()
            && !self.editor_focus
            && !self.collapsed
            && self
                .area
                .get()
                .is_some_and(|area| column == area.x && row >= area.y && row < area.bottom())
    }
    pub(crate) fn motion_active(&self, now: std::time::Instant) -> bool {
        self.editor_transition_at.get().is_some_and(|at| {
            now.saturating_duration_since(at) < std::time::Duration::from_millis(240)
        })
    }
    fn editor_motion_value(&self, now: std::time::Instant) -> f64 {
        if self.reduce_motion {
            return f64::from(self.editor_focus);
        }
        let Some(start) = self.editor_transition_at.get() else {
            return f64::from(self.editor_focus);
        };
        let t = (now.saturating_duration_since(start).as_secs_f64() / 0.24).clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        let from = self.editor_transition_from.get();
        let to = f64::from(self.editor_focus);
        from + (to - from) * eased
    }
    pub(crate) fn set_mobile_area(&self, area: Option<Rect>) {
        if area.is_some() || self.mobile_drawer.get() {
            self.area.set(area);
        }
        self.mobile_drawer.set(area.is_some());
    }
    pub(crate) fn mobile_hits(&self) -> Vec<(Rect, Action)> {
        self.hits.borrow().clone()
    }
    pub(crate) fn sync_mobile_area(&mut self, terminal: Rect, drawer_open: bool) {
        if drawer_open && regions(terminal, false).1.is_none() {
            let area = Rect::new(
                terminal.right().saturating_sub(terminal.width),
                terminal.y,
                terminal.width,
                terminal.height,
            );
            self.area.set(Some(area));
        } else if terminal.width < 100 || terminal.height < 18 {
            self.area.set(None);
            self.focused = false;
        }
    }
    pub fn inherit_controls(
        &mut self,
        permission: blora_policy::PermissionMode,
        model: &str,
        provider: &str,
    ) {
        if let Some(chat) = self.chat.as_mut() {
            chat.permission.get_or_insert(permission);
            chat.route.get_or_insert_with(|| {
                (
                    if model.is_empty() {
                        std::env::var("BLORA_MODEL").unwrap_or_default()
                    } else {
                        model.to_owned()
                    },
                    if provider.is_empty() {
                        std::env::var("BLORA_PROVIDER").unwrap_or_default()
                    } else {
                        provider.to_owned()
                    },
                )
            });
        }
    }
    pub fn control_hit(&self, x: u16, y: u16) -> Option<Control> {
        if self.active != Tab::Chat || self.area.get().is_none() || self.collapsed {
            return None;
        }
        if self.active != Tab::Chat || self.collapsed || self.area.get().is_none() {
            return None;
        }
        self.controls
            .borrow()
            .iter()
            .find(|(rect, _)| rect.contains((x, y).into()))
            .map(|(_, control)| *control)
    }
    pub fn control_chat(&self) -> Option<&SideChat> {
        match &self.control_owner {
            ControlOwner::Main => None,
            ControlOwner::Side(id) => self.chat.as_ref().filter(|chat| &chat.id == id),
        }
    }
    pub fn apply_permission(
        &mut self,
        mode: blora_policy::PermissionMode,
        main: &mut blora_policy::PermissionMode,
    ) {
        match &self.control_owner {
            ControlOwner::Main => *main = mode,
            ControlOwner::Side(id) => {
                if let Some(chat) = self.chat.as_mut().filter(|chat| &chat.id == id) {
                    chat.permission = Some(mode);
                }
            }
        }
    }
    pub fn cancel_guard(&self) -> CancelGuard {
        CancelGuard(self.cancellations.clone())
    }
    pub fn question_target<'a>(
        &'a self,
        main: Option<&'a SessionProjection>,
        current: Option<&crate::question::QuestionForm>,
        session: Option<&SessionId>,
        workspace: &Path,
    ) -> (
        bool,
        Option<SessionId>,
        PathBuf,
        Option<&'a blora_session::PendingInput>,
    ) {
        let side = self
            .chat
            .as_ref()
            .and_then(|chat| chat.projection.pending_input.as_ref())
            .is_some_and(|pending| {
                current.is_some_and(|form| form.request_id == pending.request_id)
                    || main
                        .and_then(|projection| projection.pending_input.as_ref())
                        .is_none()
            });
        if side {
            let chat = self.chat.as_ref().unwrap();
            (
                true,
                Some(chat.id.clone()),
                chat.workspace.clone(),
                chat.projection.pending_input.as_ref(),
            )
        } else {
            (
                false,
                session.cloned(),
                workspace.to_path_buf(),
                main.and_then(|projection| projection.pending_input.as_ref()),
            )
        }
    }
    pub fn approvals(
        &self,
        runtime: &Runtime,
        main: Option<&SessionId>,
    ) -> Vec<blora_storage::ApprovalRecord> {
        let primary = main
            .and_then(|id| runtime.pending_approvals(id).ok())
            .unwrap_or_default();
        let side = self
            .chat
            .as_ref()
            .and_then(|chat| runtime.pending_approvals(&chat.id).ok())
            .unwrap_or_default();
        if !side.is_empty() && (self.chat_focused() || primary.is_empty()) {
            side
        } else {
            primary
        }
    }
    pub(crate) fn sync_mentions(&mut self, sessions: &[blora_storage::SessionSummary]) {
        if let Some(chat) = self.chat.as_mut() {
            chat.mentions
                .sync(&chat.draft, chat.cursor, Some(&chat.id), sessions);
        }
    }
    pub(crate) fn mention_event(&mut self, event: &Event) -> bool {
        if self.active != Tab::Chat || self.collapsed || self.area.get().is_none() {
            return false;
        }
        if !self.focused && !matches!(event, Event::Mouse(_)) {
            return false;
        }
        if let Some(chat) = self.chat.as_mut()
            && chat
                .mentions
                .event(event, &mut chat.draft, &mut chat.cursor)
        {
            chat.history_index = None;
            self.focused = true;
            return true;
        }
        false
    }
    pub fn chat_focused(&self) -> bool {
        self.focused && self.active == Tab::Chat
    }
    pub fn chat_tool_rows(&self) -> Vec<(Rect, SessionId, Vec<usize>)> {
        if self.active != Tab::Chat || self.collapsed || self.area.get().is_none() {
            return Vec::new();
        }
        self.chat
            .as_ref()
            .map(|chat| {
                chat.tool_rows
                    .borrow()
                    .iter()
                    .map(|(rect, indices)| (*rect, chat.id.clone(), indices.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn draft_empty(&self) -> bool {
        self.chat.as_ref().is_none_or(|chat| chat.draft.is_empty())
    }
    pub fn collapsed(&self) -> bool {
        self.collapsed
    }
    pub fn completions(&self) -> Sender<(SessionId, String)> {
        self.done_tx.clone()
    }
    pub fn sync(&mut self, runtime: &Runtime, workspace: &Path, area: Rect) {
        self.mobile_drawer.set(false);
        self.area.set(self.regions(area).1);
        if self.area.get().is_none() {
            self.focused = false;
        }
        if self.tabs.contains(&Tab::Files) {
            self.files.sync(workspace);
            if self.files.preview_path.is_some() && self.files.editor.is_some() {
                let path = self.files.preview_path.clone().unwrap();
                self.files.register_active_document(path);
            }
            if matches!(self.files.pending_leave, Some(Leave::Project(_))) {
                self.active = Tab::Files;
                self.focused = true;
            }
        }
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.poll();
            if let Some(area) = self.area.get() {
                let _ =
                    terminal.resize(area.width.saturating_sub(1), area.height.saturating_sub(4));
            }
        }
        while let Ok((id, status)) = self.done_rx.try_recv() {
            if let Some(chat) = self.chat.as_mut().filter(|chat| chat.id == id) {
                chat.running = false;
                chat.cancel.cancel();
                chat.status = status;
            }
        }
        if let Some(chat) = self.chat.as_mut() {
            let _ = runtime.refresh_projection(&chat.id, &mut chat.projection);
        }
    }
    fn open(&mut self, tab: Tab, runtime: &Runtime, workspace: &Path) {
        if !self.tabs.contains(&tab) {
            self.tabs.push(tab);
            match tab {
                Tab::Terminal => {
                    self.terminal_workspace = workspace.to_path_buf();
                    match crate::right_terminal::TerminalPane::spawn(workspace, 34, 13) {
                        Ok(pane) => self.terminal = Some(pane),
                        Err(error) => self.terminal_error = error,
                    }
                }
                Tab::Chat => match runtime.create_session(CreateSession {
                    title: None,
                    workspace_path: workspace.display().to_string(),
                    mode: Mode::Code,
                    parent_session_id: None,
                }) {
                    Ok(session) => {
                        self.chat = Some(SideChat {
                            id: session,
                            workspace: workspace.to_path_buf(),
                            projection: SessionProjection::new(),
                            cancel: CancelToken::new(),
                            running: false,
                            draft: String::new(),
                            cursor: 0,
                            mentions: Default::default(),
                            paste_preview: None,
                            history: vec![],
                            history_index: None,
                            saved_draft: String::new(),
                            slash_selected: 0,
                            slash_notice: None,
                            slash_dismissed: false,
                            rendered: 0.into(),
                            max_scroll: 0.into(),
                            unread: 0.into(),
                            queue: VecDeque::new(),
                            scroll: 0.into(),
                            status: String::new(),
                            route: None,
                            permission: None,
                            cache: Default::default(),
                            tool_rows: Default::default(),
                        })
                    }
                    Err(error) => self.chat_error = error.to_string(),
                },
                _ => {}
            }
        }
        self.active = tab;
        if tab == Tab::Files {
            self.editor_focus = false;
        }
        self.focused = true;
    }
    fn restart_terminal(&mut self) {
        if self
            .terminal
            .as_ref()
            .is_some_and(|pane| pane.exit_status().is_none() && pane.error().is_none())
        {
            return;
        }
        self.terminal = None;
        self.terminal_error.clear();
        let area = self
            .area
            .get()
            .map(|area| layout::body(Rect::new(area.x + 1, area.y, area.width - 1, area.height)));
        let (cols, rows) = area
            .map(|r| (r.width, r.height.saturating_sub(1)))
            .unwrap_or((34, 13));
        match crate::right_terminal::TerminalPane::spawn(&self.terminal_workspace, cols, rows) {
            Ok(pane) => self.terminal = Some(pane),
            Err(error) => self.terminal_error = error,
        }
    }
    fn close(&mut self, tab: Tab) {
        if tab == Tab::Info {
            return;
        }
        if tab == Tab::Files && self.files.dirty() {
            self.files.pending_leave = Some(Leave::Close);
            self.active = Tab::Files;
            self.focused = true;
            return;
        }
        self.tabs.retain(|item| *item != tab);
        match tab {
            Tab::Files => self.files.close(),
            Tab::Terminal => {
                self.terminal = None;
                self.terminal_error.clear();
            }
            Tab::Chat => {
                self.chat_error.clear();
                if let Some(chat) = self.chat.take() {
                    chat.cancel.cancel();
                }
            }
            _ => {}
        }
        if self.active == tab {
            self.active = Tab::Info;
        }
        self.focused = false;
    }
    pub fn request(
        &mut self,
        permission: blora_policy::PermissionMode,
        model: &str,
        provider: &str,
        token: Option<String>,
    ) -> Option<RunRequest> {
        let chat = self.chat.as_mut()?;
        if chat.running || chat.queue.is_empty() {
            return None;
        }
        let (model, provider) = chat.route.get_or_insert_with(|| {
            let resolved_provider = if provider.is_empty() {
                std::env::var("BLORA_PROVIDER").unwrap_or_default()
            } else {
                provider.to_owned()
            };
            let resolved_model = if model.is_empty() {
                std::env::var("BLORA_MODEL").unwrap_or_default()
            } else {
                model.to_owned()
            };
            (resolved_model, resolved_provider)
        });
        if crate::passport_login_required(provider, token.as_deref()) {
            chat.status = i18n::tr("right.login", "请先在主会话 /login 登录");
            return None;
        }
        let prompt = chat.queue.pop_front()?;
        chat.cancel = CancelToken::new();
        self.cancellations
            .lock()
            .unwrap()
            .retain(|token| !token.is_cancelled());
        self.cancellations.lock().unwrap().push(chat.cancel.clone());
        chat.running = true;
        chat.status = i18n::tr("right.running", "运行中…");
        Some(RunRequest {
            id: chat.id.clone(),
            prompt,
            cancel: chat.cancel.clone(),
            options: crate::tui_run_options(
                *chat.permission.get_or_insert(permission),
                model,
                provider,
                token,
            ),
        })
    }
    pub fn event(&mut self, event: &Event, runtime: &Runtime, workspace: &Path) -> bool {
        if self.active == Tab::Files && self.files.comparison.is_some() {
            if let Event::Mouse(mouse) = event {
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    let action = self
                        .hits
                        .borrow()
                        .iter()
                        .find(|(area, _)| area.contains((mouse.column, mouse.row).into()))
                        .map(|(_, action)| action.clone());
                    match action {
                        Some(Action::CompareKeep) => {
                            self.files.comparison = None;
                            return true;
                        }
                        Some(Action::CompareReload) => {
                            self.files.reload_compared();
                            return true;
                        }
                        _ => {}
                    }
                }
            }
            match event {
                Event::Key(key)
                    if self.focused
                        && matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    match key.code {
                        KeyCode::Esc => self.files.comparison = None,
                        KeyCode::Char('r') => self.files.reload_compared(),
                        KeyCode::Down | KeyCode::PageDown => {
                            let comparison = self.files.comparison.as_mut().unwrap();
                            comparison.scroll = (comparison.scroll
                                + if key.code == KeyCode::Down { 1 } else { 10 })
                            .min(comparison.lines.len().saturating_sub(1));
                        }
                        KeyCode::Up | KeyCode::PageUp => {
                            let comparison = self.files.comparison.as_mut().unwrap();
                            comparison.scroll = comparison
                                .scroll
                                .saturating_sub(if key.code == KeyCode::Up { 1 } else { 10 });
                        }
                        _ => {}
                    }
                    return true;
                }
                Event::Mouse(mouse)
                    if self
                        .area
                        .get()
                        .is_some_and(|area| area.contains((mouse.column, mouse.row).into())) =>
                {
                    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                        self.focused = true;
                    }
                    let comparison = self.files.comparison.as_mut().unwrap();
                    match mouse.kind {
                        MouseEventKind::ScrollDown => {
                            comparison.scroll = (comparison.scroll + 3)
                                .min(comparison.lines.len().saturating_sub(1))
                        }
                        MouseEventKind::ScrollUp => {
                            comparison.scroll = comparison.scroll.saturating_sub(3)
                        }
                        _ => {}
                    }
                    return true;
                }
                Event::Paste(_) if self.focused => return true,
                _ => {}
            }
        }
        if let Event::Mouse(mouse) = event {
            if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
                self.editor_dragging = false;
            }
            if self.editor_dragging
                && matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left))
                && self.active == Tab::Files
                && self.files.pending_leave.is_none()
            {
                if let (Some(area), Some(editor)) =
                    (self.editor_area.get(), self.files.editor.as_mut())
                {
                    if area.width > 0 && area.height > 0 {
                        if mouse.row < area.y {
                            self.files.scroll = self.files.scroll.saturating_sub(1);
                        }
                        if mouse.row >= area.bottom() {
                            self.files.scroll =
                                (self.files.scroll + 1).min(self.files.max_scroll.get());
                        }
                        if mouse.column < area.x {
                            self.files.horizontal = self.files.horizontal.saturating_sub(1);
                        }
                        if mouse.column >= area.right() {
                            self.files.horizontal =
                                (self.files.horizontal + 1).min(self.files.max_horizontal.get());
                        }
                        let row = mouse.row.clamp(area.y, area.bottom() - 1) - area.y;
                        let col = mouse.column.clamp(area.x, area.right() - 1) - area.x;
                        editor.move_to_display_position(
                            self.files.scroll + row as usize,
                            self.files.horizontal + col as usize,
                        );
                    }
                }
                return true;
            }
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.editor_dragging = false;
                if self.active == Tab::Files && self.files.pending_leave.is_some() {
                    let allowed = self
                        .hits
                        .borrow()
                        .iter()
                        .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                        .is_some_and(|(_, action)| {
                            matches!(action, Action::FileConfirm(_) | Action::FileCancel)
                        });
                    if !allowed {
                        return true;
                    }
                }
                if self.active == Tab::Files {
                    let clicked = self
                        .hits
                        .borrow()
                        .iter()
                        .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                        .map(|(_, action)| action.clone());
                    match clicked {
                        Some(Action::Document(path)) => {
                            self.files.activate_path(&path);
                            self.explorer_focused = false;
                            self.focused = true;
                            return true;
                        }
                        Some(Action::CloseDocument(path)) => {
                            if self.files.documents.iter().any(|d| {
                                d.path == path
                                    && d.editor
                                        .as_ref()
                                        .is_some_and(crate::right_panel::editor::Editor::dirty)
                            }) {
                                self.files.pending_leave = Some(Leave::CloseDocument(path));
                            } else if let Some(i) =
                                self.files.documents.iter().position(|d| d.path == path)
                            {
                                self.files.close_document(i);
                            }
                            return true;
                        }
                        _ => {}
                    }
                }
                if self.active == Tab::Files
                    && self.files.editor.is_some()
                    && self
                        .editor_area
                        .get()
                        .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
                {
                    let area = self.editor_area.get().expect("checked editor area");
                    self.focused = true;
                    self.explorer_focused = false;
                    let row = mouse.row.saturating_sub(area.y) as usize + self.files.scroll;
                    let column =
                        mouse.column.saturating_sub(area.x) as usize + self.files.horizontal;
                    if let Some(editor) = self.files.editor.as_mut() {
                        editor.begin_mouse_selection(
                            row,
                            column,
                            mouse.modifiers.contains(KeyModifiers::SHIFT),
                        );
                        self.editor_dragging = true;
                    }
                    return true;
                }
                if self
                    .chat_tool_rows()
                    .iter()
                    .any(|(rect, _, _)| rect.contains((mouse.column, mouse.row).into()))
                {
                    self.focused = true;
                    return false;
                }
                let hit = {
                    self.hits
                        .borrow()
                        .iter()
                        .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                        .cloned()
                };
                if let Some((_, action)) = hit {
                    if self.active == Tab::Files
                        && self.files.pending_leave.is_some()
                        && !matches!(action, Action::FileConfirm(_) | Action::FileCancel)
                    {
                        return true;
                    }
                    if !matches!(
                        action,
                        Action::Close(_) | Action::Collapse | Action::Restore
                    ) {
                        self.focused = true;
                    }
                    match action {
                        Action::Tab(tab) => self.open(tab, runtime, workspace),
                        Action::Close(tab) => self.close(tab),
                        Action::Collapse => {
                            self.collapsed = true;
                            self.focused = false;
                        }
                        Action::Restore => self.collapsed = false,
                        Action::File(index) => {
                            self.focused = true;
                            if self.explorer_area.get().is_some() {
                                self.explorer_focused = self
                                    .files
                                    .entries
                                    .get(index)
                                    .is_some_and(|entry| entry.directory);
                                self.files.open_in_explorer(index);
                            } else {
                                self.explorer_focused = false;
                                self.files.open(index);
                            }
                        }
                        Action::Document(path) => {
                            self.files.activate_path(&path);
                        }
                        Action::CloseDocument(path) => {
                            if self.files.documents.iter().any(|d| {
                                d.path == path
                                    && d.editor
                                        .as_ref()
                                        .is_some_and(crate::right_panel::editor::Editor::dirty)
                            }) {
                                self.files.pending_leave = Some(Leave::CloseDocument(path));
                            } else if let Some(i) =
                                self.files.documents.iter().position(|d| d.path == path)
                            {
                                self.files.close_document(i);
                            }
                        }
                        Action::FileSave => {
                            self.files.save();
                        }
                        Action::CompareKeep => self.files.comparison = None,
                        Action::CompareReload => self.files.reload_compared(),
                        Action::FileConfirm(save) => self.confirm_file_leave(save),
                        Action::FileCancel => self.files.cancel_leave(),
                        Action::Back => self.files.back(),
                        Action::Explorer => self.files.show_explorer(),
                        Action::DocumentCycle(backwards) => self.files.cycle_document(backwards),
                        Action::Focus => self.focused = true,
                        Action::Refresh => self.files.refresh(),
                        Action::Restart => self.restart_terminal(),
                        Action::ExpandId => self.expanded_id = !self.expanded_id,
                        Action::Send => {
                            if let Some(chat) = self.chat.as_mut() {
                                chat.send();
                            }
                        }
                        Action::Cancel => {
                            if let Some(chat) = self.chat.as_mut() {
                                chat.cancel.cancel();
                                chat.queue.clear();
                            }
                        }
                        Action::Bottom => {
                            if let Some(chat) = self.chat.as_ref() {
                                chat.scroll.set(0);
                                chat.unread.set(0);
                            }
                        }
                    }
                    return true;
                }
                if self.active == Tab::Files && self.files.pending_leave.is_some() {
                    return true;
                }
                self.focused = false;
            }
            if self
                .area
                .get()
                .is_some_and(|rect| rect.contains((mouse.column, mouse.row).into()))
            {
                let delta = match mouse.kind {
                    MouseEventKind::ScrollDown => 3isize,
                    MouseEventKind::ScrollUp => -3,
                    _ => return true,
                };
                match self.active {
                    Tab::Files
                        if self.explorer_area.get().is_some_and(|rect| {
                            rect.contains((mouse.column, mouse.row).into())
                        }) =>
                    {
                        self.files.selected = self
                            .files
                            .selected
                            .saturating_add_signed(delta)
                            .min(self.files.entries.len().saturating_sub(1));
                    }
                    Tab::Files if self.files.editor.is_some() => {
                        if self
                            .editor_area
                            .get()
                            .is_some_and(|rect| rect.contains((mouse.column, mouse.row).into()))
                        {
                            self.files.scroll = self
                                .files
                                .scroll
                                .saturating_add_signed(delta)
                                .min(self.files.max_scroll.get());
                        }
                    }
                    Tab::Info => self.info_scroll.set(
                        self.info_scroll
                            .get()
                            .saturating_add_signed(delta)
                            .min(self.info_max.get()),
                    ),
                    Tab::Files => {
                        if self.files.preview.is_some() {
                            self.files.scroll = self
                                .files
                                .scroll
                                .saturating_add_signed(delta)
                                .min(self.files.max_scroll.get());
                        } else {
                            self.files.selected = self
                                .files
                                .selected
                                .saturating_add_signed(delta)
                                .min(self.files.entries.len().saturating_sub(1));
                        }
                    }
                    Tab::Chat => {
                        if let Some(chat) = self.chat.as_mut() {
                            chat.scroll_page(delta > 0, delta.unsigned_abs() as usize);
                        }
                    }
                    Tab::Terminal => {
                        if let Some(terminal) = self.terminal.as_mut() {
                            let content = self.area.get().map(|area| {
                                let inner =
                                    Rect::new(area.x + 1, area.y, area.width - 1, area.height);
                                let body = layout::body(inner);
                                Rect::new(
                                    body.x,
                                    body.y + 1,
                                    body.width,
                                    body.height.saturating_sub(1),
                                )
                            });
                            if let Some(content) = content
                                .filter(|rect| rect.contains((mouse.column, mouse.row).into()))
                            {
                                if terminal.screen().mouse_protocol_mode()
                                    != vt100::MouseProtocolMode::None
                                    && terminal.history_offset() == 0
                                    && !mouse.modifiers.contains(KeyModifiers::SHIFT)
                                {
                                    let button = if delta < 0 { 64 } else { 65 };
                                    let col = mouse.column - content.x + 1;
                                    let row = mouse.row - content.y + 1;
                                    match terminal.screen().mouse_protocol_encoding() {
                                        vt100::MouseProtocolEncoding::Sgr => {
                                            let _ = terminal.input(
                                                format!("\x1b[<{button};{col};{row}M").as_bytes(),
                                            );
                                        }
                                        vt100::MouseProtocolEncoding::Default
                                            if col < 224 && row < 224 =>
                                        {
                                            let _ = terminal.input(&[
                                                27,
                                                b'[',
                                                b'M',
                                                button + 32,
                                                col as u8 + 32,
                                                row as u8 + 32,
                                            ]);
                                        }
                                        _ => {}
                                    }
                                } else {
                                    terminal.scroll_history(-delta);
                                }
                            }
                        }
                    }
                }
                return true;
            }
            return false;
        }
        if !self.focused || self.area.get().is_none() {
            return false;
        }
        if self.active == Tab::Files && self.files.pending_leave.is_some() {
            if let Event::Key(key) = event {
                if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    match key.code {
                        KeyCode::Char('s') => self.confirm_file_leave(true),
                        KeyCode::Char('d') => self.confirm_file_leave(false),
                        KeyCode::Char('c') | KeyCode::Esc => self.files.cancel_leave(),
                        _ => {}
                    }
                }
            }
            return true;
        }
        if self.active == Tab::Files
            && self
                .files
                .editor
                .as_ref()
                .is_some_and(|editor| editor.has_find() || editor.goto_is_open())
        {
            if let Some(editor) = self.files.editor.as_mut() {
                let goto_open = editor.goto_is_open();
                match event {
                    Event::Paste(text) => {
                        if goto_open {
                            editor.goto_paste(text);
                        } else {
                            editor.find_paste(text);
                        }
                    }
                    Event::Key(key)
                        if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                    {
                        if goto_open {
                            if editor.goto_key(*key) == editor::GotoAction::Jump {
                                self.files.scroll = editor.position().0;
                            }
                        } else {
                            match editor.find_key(*key) {
                                editor::FindAction::Next | editor::FindAction::Prev => {
                                    self.files.scroll = editor.position().0;
                                }
                                editor::FindAction::Replace => {
                                    self.files.message = "已替换当前匹配".into();
                                    self.files.scroll = editor.position().0;
                                }
                                editor::FindAction::ReplaceAll => {
                                    self.files.message = "已替换全部匹配".into();
                                    self.files.scroll = editor.position().0;
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            return true;
        }
        if let Event::Key(key) = event {
            if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                return true;
            }
            if self.active == Tab::Files && key.code == KeyCode::F(5) && self.files.editor.is_some()
            {
                self.files.compare_disk();
                return true;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                if self.active == Tab::Files
                    && matches!(
                        key.code,
                        KeyCode::Char('p') | KeyCode::Tab | KeyCode::BackTab
                    )
                    && !self.files.documents.is_empty()
                {
                    let n = self.files.documents.len();
                    let i = self.files.active_document.unwrap_or(0);
                    let next = if key.modifiers.contains(KeyModifiers::SHIFT)
                        || key.code == KeyCode::BackTab
                    {
                        (i + n - 1) % n
                    } else {
                        (i + 1) % n
                    };
                    self.files.activate_document(next);
                    return true;
                }
                if self.active == Tab::Files
                    && key.code == KeyCode::Char('g')
                    && self.files.editor.is_some()
                {
                    self.files.editor.as_mut().unwrap().key(*key);
                    return true;
                }
            }
            if self.active == Tab::Files && key.code == KeyCode::F(11) {
                self.editor_transition_from
                    .set(self.editor_motion_value(std::time::Instant::now()));
                self.editor_focus = !self.editor_focus;
                if !self.reduce_motion {
                    self.editor_transition_at
                        .set(Some(std::time::Instant::now()));
                }
                return true;
            }
            if self.active == Tab::Files && key.code == KeyCode::F(6) {
                if self.explorer_area.get().is_some() {
                    self.explorer_focused = !self.explorer_focused;
                } else {
                    self.files.show_explorer();
                }
                return true;
            }
            if self.active == Tab::Files && self.explorer_focused {
                // Resizing can remove the split explorer between key events.
                if self.explorer_area.get().is_none() {
                    self.explorer_focused = false;
                }
            }
            if self.active == Tab::Files && self.explorer_focused {
                match key.code {
                    KeyCode::Left => self.files.tree_horizontal(false),
                    KeyCode::Right => self.files.tree_horizontal(true),
                    KeyCode::Home => self.files.selected = 0,
                    KeyCode::End => {
                        self.files.selected = self.files.entries.len().saturating_sub(1)
                    }
                    KeyCode::Up => self.files.selected = self.files.selected.saturating_sub(1),
                    KeyCode::Down => {
                        self.files.selected = (self.files.selected + 1)
                            .min(self.files.entries.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        let directory = self
                            .files
                            .entries
                            .get(self.files.selected)
                            .is_some_and(|entry| entry.directory);
                        self.files.open_in_explorer(self.files.selected);
                        self.explorer_focused = directory;
                    }
                    KeyCode::Esc => self.explorer_focused = false,
                    _ => {}
                }
                return true;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('g') => {
                        self.focused = false;
                        return true;
                    }
                    KeyCode::Char('q') if self.files.dirty() => {
                        self.guard_file_leave();
                        return true;
                    }
                    KeyCode::Char('q') => return false,
                    KeyCode::Char('w') => {
                        self.close(self.active);
                        return true;
                    }
                    KeyCode::Char('c') if self.active == Tab::Chat => {
                        if let Some(chat) = self.chat.as_mut() {
                            chat.cancel.cancel();
                            chat.queue.clear();
                        }
                        return true;
                    }
                    _ => {}
                }
            }
            if key.code == KeyCode::Tab
                && !(self.active == Tab::Files && self.files.editor.is_some())
                && (self.active != Tab::Terminal || key.modifiers.contains(KeyModifiers::CONTROL))
            {
                let index = self
                    .tabs
                    .iter()
                    .position(|tab| *tab == self.active)
                    .unwrap_or(0);
                self.active = self.tabs[(index + 1) % self.tabs.len()];
                return true;
            }
        }
        if self.active == Tab::Chat {
            if let Event::Key(key) = event {
                if let KeyCode::Char(c) = key.code
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    if let Some(chat) = self.chat.as_mut() {
                        chat.insert(&c.to_string());
                        return true;
                    }
                }
            }
        }
        match self.active {
            Tab::Terminal => {
                if let Some(terminal) = self.terminal.as_mut() {
                    match event {
                        Event::Paste(text) => {
                            terminal.follow_output();
                            let _ = terminal.paste(text);
                        }
                        Event::Key(key)
                            if key.modifiers.contains(KeyModifiers::SHIFT)
                                && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) =>
                        {
                            let page = self
                                .area
                                .get()
                                .map(|area| area.height.saturating_sub(5) as isize)
                                .unwrap_or(10);
                            terminal.scroll_history(if key.code == KeyCode::PageUp {
                                page
                            } else {
                                -page
                            });
                        }
                        Event::Key(key)
                            if key.code == KeyCode::End && terminal.history_offset() > 0 =>
                        {
                            terminal.follow_output()
                        }
                        Event::Key(key) => {
                            terminal.follow_output();
                            let _ = terminal.key(*key);
                        }
                        _ => {}
                    }
                }
            }
            Tab::Files => {
                if let Event::Key(key) = event {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('s')
                    {
                        self.files.save();
                        return true;
                    }
                    if key.code == KeyCode::Esc {
                        self.files.back();
                        return true;
                    }
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        if let Some(text) =
                            self.files.editor.as_ref().and_then(|e| e.selected_text())
                        {
                            crate::copy_text_to_clipboard(text);
                        }
                        return true;
                    }
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('x')
                    {
                        if let Some(editor) = self.files.editor.as_mut() {
                            let text = editor.cut();
                            if !text.is_empty() {
                                crate::copy_text_to_clipboard(&text);
                                self.files.preview = Some(editor.text().to_owned());
                            }
                        }
                        return true;
                    }
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('v')
                    {
                        if let Some(text) = crate::read_clipboard_text() {
                            if let Some(editor) = self.files.editor.as_mut() {
                                editor.insert(&text);
                                self.files.preview = Some(editor.text().to_owned());
                            }
                        }
                        return true;
                    }
                    if key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT) {
                        if let Some(e) = self.files.editor.as_mut() {
                            e.indent_lines(true);
                        }
                        return true;
                    }
                }
                if let Some(editor) = self.files.editor.as_mut() {
                    match event {
                        Event::Paste(text) => {
                            editor.insert(text);
                        }
                        Event::Key(key) => editor.key(*key),
                        _ => {}
                    }
                    self.files.preview = Some(editor.text().to_owned());
                    let (row, _) = editor.position();
                    let page = self
                        .area
                        .get()
                        .map_or(10, |r| r.height.saturating_sub(7) as usize)
                        .max(1);
                    if row < self.files.scroll {
                        self.files.scroll = row;
                    }
                    if row >= self.files.scroll + page {
                        self.files.scroll = row + 1 - page;
                    }
                    let column = unicode_width::UnicodeWidthStr::width(
                        editor.text()[..editor.cursor()]
                            .rsplit('\n')
                            .next()
                            .unwrap_or("")
                            .replace('\t', "    ")
                            .as_str(),
                    );
                    let width = self
                        .area
                        .get()
                        .map_or(24, |r| r.width.saturating_sub(8) as usize)
                        .max(1);
                    if column < self.files.horizontal {
                        self.files.horizontal = column;
                    }
                    if column >= self.files.horizontal + width {
                        self.files.horizontal = column + 1 - width;
                    }
                } else {
                    if let Event::Key(key) = event {
                        let page = self
                            .area
                            .get()
                            .map(|r| r.height.saturating_sub(6) as usize)
                            .unwrap_or(10)
                            .max(1);
                        let preview = self.files.preview_path.is_some();
                        let delta = match key.code {
                            KeyCode::Up => -1,
                            KeyCode::Down => 1,
                            KeyCode::PageUp => -(page as isize),
                            KeyCode::PageDown => page as isize,
                            _ => 0,
                        };
                        if preview {
                            self.files.scroll = self
                                .files
                                .scroll
                                .saturating_add_signed(delta)
                                .min(self.files.max_scroll.get());
                        } else {
                            self.files.selected = self
                                .files
                                .selected
                                .saturating_add_signed(delta)
                                .min(self.files.entries.len().saturating_sub(1));
                        }
                        match key.code {
                            KeyCode::Home => {
                                if preview {
                                    self.files.scroll = 0;
                                    self.files.horizontal = 0;
                                } else {
                                    self.files.selected = 0;
                                }
                            }
                            KeyCode::End => {
                                if preview {
                                    self.files.scroll = self.files.max_scroll.get();
                                } else {
                                    self.files.selected =
                                        self.files.entries.len().saturating_sub(1);
                                }
                            }
                            KeyCode::Left if preview => {
                                self.files.horizontal = self.files.horizontal.saturating_sub(4)
                            }
                            KeyCode::Right if preview => {
                                self.files.horizontal =
                                    (self.files.horizontal + 4).min(self.files.max_horizontal.get())
                            }
                            KeyCode::Char('r') => self.files.refresh(),
                            KeyCode::Enter if !preview => self.files.open(self.files.selected),
                            KeyCode::Backspace | KeyCode::Esc => self.files.back(),
                            _ => {}
                        }
                    }
                }
            }
            Tab::Chat => {
                if let Some(chat) = self.chat.as_mut() {
                    if let Event::Key(key) = event
                        && !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                    {
                        return true;
                    }
                    match event {
                        Event::Paste(text) => {
                            let tail = chat.draft.split_off(chat.cursor);
                            crate::insert_pasted_text(
                                &mut chat.draft,
                                &mut chat.paste_preview,
                                &text,
                            );
                            if chat.paste_preview.is_some() {
                                chat.draft.push_str(&tail);
                                chat.cursor = chat.draft.len();
                            } else {
                                chat.cursor = chat.draft.len();
                                chat.draft.push_str(&tail);
                            }
                            chat.history_index = None;
                            chat.slash_dismissed = false;
                            chat.slash_notice = None;
                        }
                        Event::Key(key) if key.code == KeyCode::Insert => {
                            let tail = chat.draft.split_off(chat.cursor);
                            crate::paste_clipboard(&mut chat.draft, &mut chat.paste_preview);
                            if chat.paste_preview.is_some() {
                                chat.draft.push_str(&tail);
                                chat.cursor = chat.draft.len();
                            } else {
                                chat.cursor = chat.draft.len();
                                chat.draft.push_str(&tail);
                            }
                            chat.history_index = None;
                            chat.slash_dismissed = false;
                            chat.slash_notice = None;
                        }
                        Event::Key(key)
                            if key.code == KeyCode::Char('v')
                                && key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            let tail = chat.draft.split_off(chat.cursor);
                            crate::paste_clipboard(&mut chat.draft, &mut chat.paste_preview);
                            if chat.paste_preview.is_some() {
                                chat.draft.push_str(&tail);
                                chat.cursor = chat.draft.len();
                            } else {
                                chat.cursor = chat.draft.len();
                                chat.draft.push_str(&tail);
                            }
                            chat.history_index = None;
                            chat.slash_dismissed = false;
                            chat.slash_notice = None;
                        }
                        Event::Key(key) if key.code == KeyCode::PageUp => {
                            chat.scroll_page(false, 8)
                        }
                        Event::Key(key) if key.code == KeyCode::PageDown => {
                            chat.scroll_page(true, 8)
                        }
                        Event::Key(key) if key.code == KeyCode::Home => {
                            chat.scroll.set(chat.max_scroll.get());
                            chat.unread.set(0);
                        }
                        Event::Key(key) if key.code == KeyCode::End => {
                            chat.scroll.set(0);
                            chat.unread.set(0);
                        }
                        Event::Key(key)
                            if crate::slash::is_open(&chat.draft) && key.code == KeyCode::Up =>
                        {
                            chat.slash_selected = chat.slash_selected.saturating_sub(1);
                        }
                        Event::Key(key)
                            if crate::slash::is_open(&chat.draft) && key.code == KeyCode::Down =>
                        {
                            let count = crate::slash::matches(&chat.draft).len();
                            if count > 0 {
                                chat.slash_selected = (chat.slash_selected + 1).min(count - 1);
                            }
                        }
                        Event::Key(key)
                            if crate::slash::is_open(&chat.draft) && key.code == KeyCode::Tab =>
                        {
                            if let Some(command) =
                                crate::slash::matches(&chat.draft).get(chat.slash_selected)
                            {
                                chat.draft = crate::slash::complete(command);
                                chat.cursor = chat.draft.len();
                                chat.slash_selected = 0;
                                chat.slash_notice = None;
                            }
                        }
                        Event::Key(key)
                            if key.code == KeyCode::Esc && crate::slash::is_open(&chat.draft) =>
                        {
                            chat.slash_notice = None;
                            chat.slash_selected = 0;
                            // Keep the draft, but suppress this query until it changes.
                            chat.slash_dismissed = true;
                        }
                        Event::Key(key)
                            if key.code == KeyCode::Enter && crate::slash::is_open(&chat.draft) =>
                        {
                            let hits = crate::slash::matches(&chat.draft);
                            if let Some(command) = hits.get(chat.slash_selected) {
                                if crate::slash::needs_args(command) {
                                    chat.draft = crate::slash::complete(command);
                                    chat.cursor = chat.draft.len();
                                    chat.slash_selected = 0;
                                } else {
                                    chat.run_command(runtime, &format!("/{}", command.name));
                                }
                            } else {
                                chat.slash_notice = Some("没有匹配的侧聊命令".to_owned());
                            }
                        }
                        Event::Key(key)
                            if key.code == KeyCode::Enter && chat.draft.starts_with('/') =>
                        {
                            chat.run_command(runtime, &chat.draft.clone());
                        }
                        Event::Key(key)
                            if key.code == KeyCode::Esc
                                && (chat.paste_preview.is_some()
                                    || chat.slash_notice.is_some()) =>
                        {
                            chat.paste_preview = None;
                            chat.slash_notice = None;
                        }
                        Event::Key(key) => chat.edit(*key),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        true
    }
    pub fn guard_file_leave(&mut self) -> bool {
        if !self.files.dirty() {
            return true;
        }
        self.files.pending_leave = Some(Leave::Close);
        self.active = Tab::Files;
        self.focused = true;
        self.collapsed = false;
        false
    }
    fn confirm_file_leave(&mut self, save: bool) {
        let Some(leave) = self.files.confirm(save) else {
            return;
        };
        match leave {
            Leave::Back => self.files.back(),
            Leave::Refresh => self.files.refresh(),
            Leave::Close => self.close(Tab::Files),
            Leave::Project(root) => self.files.sync(&root),
            Leave::CloseDocument(_) => {}
        }
    }
    pub fn draw(&self, frame: &mut Frame<'_>, model: &FrameModel<'_>, canvas: Rect) {
        let theme = Theme::current();
        let mut hits = self.hits.borrow_mut();
        hits.clear();
        self.controls.borrow_mut().clear();
        let restore = Rect::new(canvas.right().saturating_sub(4), canvas.y, 4, 1);
        if self.area.get().is_none() {
            frame.render_widget(
                Paragraph::new("[◧]").style(theme.fg(theme.text_dim)),
                restore,
            );
            hits.push((restore, Action::Restore));
            return;
        }
        let area = self.area.get().unwrap();
        let motion = self.editor_motion_value(std::time::Instant::now());
        let full_editor = self.editor_focus && self.active == Tab::Files;
        let editor_area = if self.active == Tab::Files
            && !self.reduce_motion
            && self.editor_transition_at.get().is_some()
        {
            let width = (f64::from(area.width) * (1.0 - motion) + f64::from(canvas.width) * motion)
                .round()
                .clamp(1.0, f64::from(canvas.width)) as u16;
            self.editor_progress.set(motion);
            if self
                .editor_transition_at
                .get()
                .is_some_and(|at| at.elapsed() >= std::time::Duration::from_millis(240))
            {
                self.editor_transition_at.set(None);
            }
            Rect::new(
                canvas.right().saturating_sub(width),
                area.y,
                width,
                area.height,
            )
        } else {
            if full_editor {
                Rect::new(canvas.x, area.y, canvas.width, area.height)
            } else {
                area
            }
        };
        if !full_editor {
            frame.render_widget(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(theme.fg(theme.hairline))
                    .style(theme.base()),
                editor_area,
            );
        }
        let mobile = self.mobile_drawer.get();
        let inner = if full_editor {
            editor_area
        } else if mobile {
            editor_area
        } else {
            Rect::new(
                editor_area.x + 1,
                editor_area.y,
                editor_area.width.saturating_sub(1),
                editor_area.height,
            )
        };
        let tab_inner = if full_editor {
            Rect::new(inner.x, inner.y, 0, 0)
        } else if mobile {
            Rect::new(inner.x, inner.y, 0, 0)
        } else {
            inner
        };
        let tab_rects = if full_editor || mobile {
            Vec::new()
        } else {
            layout::tabs(tab_inner, self.tabs.len())
        };
        for (tab, rect) in self.tabs.iter().zip(tab_rects) {
            let selected = *tab == self.active;
            let hovered = layout::hovered(rect, model.pointer);
            let style = if selected {
                theme.fg(theme.rose).bg(theme.hairline)
            } else if hovered {
                theme.fg(theme.text).bg(theme.bg_raised)
            } else {
                theme.fg(theme.text_dim)
            };
            let style = if selected && self.focused {
                style.add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                style
            };
            let reserve = if *tab == Tab::Info { 0 } else { 2 };
            frame.render_widget(
                Paragraph::new(layout::clip(
                    &if *tab == Tab::Chat {
                        self.chat
                            .as_ref()
                            .map(SideChat::title)
                            .unwrap_or_else(|| tab.label())
                    } else {
                        tab.label()
                    },
                    rect.width.saturating_sub(reserve) as usize,
                ))
                .style(style),
                rect,
            );
            hits.push((rect, Action::Tab(*tab)));
            if mobile {
                let close = Rect::new(rect.right().saturating_sub(1), rect.y, 1, 1);
                frame.render_widget(Paragraph::new("×").style(style), close);
                hits.insert(0, (close, Action::Close(*tab)));
            } else if *tab != Tab::Info && (selected || hovered) && rect.width >= 2 {
                let close = Rect::new(rect.right() - 1, rect.y, 1, 1);
                frame.render_widget(Paragraph::new("×").style(style), close);
                hits.insert(0, (close, Action::Close(*tab)));
            }
        }
        if !full_editor && !mobile {
            let collapse = Rect::new(inner.right() - 2, inner.y, 2, 1);
            frame.render_widget(
                Paragraph::new("›").style(theme.fg(theme.text_dim)),
                collapse,
            );
            hits.insert(0, (collapse, Action::Collapse));
        }
        let hint = if self.focused {
            i18n::tr("right.focused", "● 焦点 · Ctrl+G 返回 · Ctrl+W 关闭")
        } else {
            i18n::tr("right.focus", "点击聚焦 · Ctrl+Q 退出")
        };
        if !full_editor {
            frame.render_widget(
                Paragraph::new(hint).style(theme.fg(theme.text_mute)),
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            );
        }
        let body = if full_editor || mobile {
            inner
        } else {
            layout::body(inner)
        };
        if mobile && body.height > 0 {
            let title = Rect::new(body.x, body.y, body.width, 1);
            frame.render_widget(
                Paragraph::new(format!("{}  ×", self.active.label()))
                    .style(theme.fg(theme.rose).bg(theme.hairline)),
                title,
            );
            hits.push((title, Action::Collapse));
        }
        hits.push((body, Action::Focus));
        match self.active {
            Tab::Info => {
                let unknown = i18n::tr("right.unknown", "未知");
                let session = model.sessions.get(model.index);
                let p = model.projection;
                let harness = p.map(|p| p.harness).unwrap_or_default();
                let external = harness != blora_types::HarnessKind::Blora;
                let title = p
                    .and_then(|projection| projection.session.as_ref())
                    .and_then(|session| session.title.as_deref())
                    .or_else(|| session.and_then(|s| s.title.as_deref()))
                    .map(str::trim)
                    .filter(|t| !t.is_empty() && *t != "tui")
                    .map(str::to_owned)
                    .unwrap_or_else(|| i18n::tr("session.untitled", "未命名会话"));
                let id = session
                    .map(|s| s.id.to_string())
                    .unwrap_or_else(|| unknown.clone());
                let shown_id = if self.expanded_id {
                    id.clone()
                } else {
                    format!("{}… [↔]", layout::clip(&id, 8))
                };
                let provider = if external {
                    i18n::tr("right.external_provider", "由外部内核管理")
                } else if !model.provider.is_empty() {
                    model.provider.to_owned()
                } else {
                    p.and_then(|p| p.provider.clone())
                        .unwrap_or_else(|| unknown.clone())
                };
                let model_name = if external {
                    p.and_then(|p| p.external_model.clone())
                        .unwrap_or_else(|| unknown.clone())
                } else if !model.model.is_empty() {
                    model.model.to_owned()
                } else {
                    p.and_then(|p| p.model.clone())
                        .unwrap_or_else(|| unknown.clone())
                };
                let mut lines = Vec::new();
                let mut id_rows = (0, 0);
                let sections = [
                    (
                        i18n::tr("right.section_session", "会话"),
                        vec![
                            (i18n::tr("right.title", "会话"), title),
                            ("ID".into(), shown_id),
                            (
                                i18n::tr("right.project", "项目"),
                                model.workspace.display().to_string(),
                            ),
                            (
                                i18n::tr("right.state", "状态"),
                                if model.running {
                                    i18n::tr("right.running", "运行中…")
                                } else {
                                    i18n::tr("right.idle", "空闲")
                                },
                            ),
                        ],
                    ),
                    (
                        i18n::tr("right.section_config", "运行配置"),
                        vec![
                            (
                                i18n::tr("right.kernel", "内核"),
                                harness.display_name().to_owned(),
                            ),
                            (i18n::tr("right.provider", "供应商"), provider),
                            (i18n::tr("right.model", "模型"), model_name),
                            (
                                i18n::tr("right.permission", "权限"),
                                if external {
                                    i18n::tr(
                                        "right.external_permission",
                                        "由外部内核执行；主窗口控制授权",
                                    )
                                } else {
                                    i18n::permission_name(model.permission_mode)
                                },
                            ),
                        ],
                    ),
                    (
                        i18n::tr("right.section_usage", "累计用量"),
                        vec![
                            (
                                i18n::tr("right.input_tokens", "累计输入 Token"),
                                p.map(|p| p.input_tokens).unwrap_or(0).to_string(),
                            ),
                            (
                                i18n::tr("right.output_tokens", "累计输出 Token"),
                                p.map(|p| p.output_tokens).unwrap_or(0).to_string(),
                            ),
                            (i18n::tr("right.context", "上下文"), unknown),
                        ],
                    ),
                ];
                for (section, fields) in sections {
                    if !lines.is_empty() {
                        lines.push(ratatui::text::Line::default());
                    }
                    lines.push(ratatui::text::Line::styled(section, theme.fg(theme.rose)));
                    for (label, value) in fields {
                        lines.push(ratatui::text::Line::styled(
                            label.clone(),
                            theme.fg(theme.text_dim),
                        ));
                        let begin = lines.len();
                        let (wrapped, _) =
                            chat::draft_lines(&value, value.len(), body.width.max(1) as usize);
                        lines.extend(
                            wrapped.into_iter().map(|line| {
                                ratatui::text::Line::styled(line, theme.fg(theme.text))
                            }),
                        );
                        if label == "ID" {
                            id_rows = (begin, lines.len());
                        }
                    }
                }
                let action_height = body.height.min(3);
                let info = Rect::new(
                    body.x,
                    body.y,
                    body.width,
                    body.height.saturating_sub(action_height),
                );
                let max = lines.len().saturating_sub(info.height as usize);
                self.info_max.set(max);
                let offset = self.info_scroll.get().min(max);
                self.info_scroll.set(offset);
                frame.render_widget(
                    Paragraph::new(lines).scroll((offset.min(u16::MAX as usize) as u16, 0)),
                    info,
                );
                for row in id_rows.0..id_rows.1 {
                    if row >= offset && row - offset < info.height as usize {
                        hits.insert(
                            0,
                            (
                                Rect::new(info.x, info.y + (row - offset) as u16, info.width, 1),
                                Action::ExpandId,
                            ),
                        );
                    }
                }
                for (i, tab) in [Tab::Files, Tab::Terminal, Tab::Chat]
                    .iter()
                    .take(action_height as usize)
                    .enumerate()
                {
                    let rect = Rect::new(body.x, info.bottom() + i as u16, body.width, 1);
                    let label = if *tab == Tab::Chat {
                        i18n::tr("right.session_title", "侧边对话")
                    } else {
                        tab.label()
                    };
                    let style = if layout::hovered(rect, model.pointer) {
                        theme.fg(theme.rose).bg(theme.hairline)
                    } else {
                        theme.fg(theme.rose)
                    };
                    frame.render_widget(Paragraph::new(format!("[ {label} ]")).style(style), rect);
                    hits.insert(0, (rect, Action::Tab(*tab)));
                }
            }
            Tab::Files => {
                self.editor_area.set(self.draw_files(
                    frame,
                    model.pointer,
                    body,
                    &mut hits,
                    self.editor_focus,
                ));
            }
            Tab::Terminal => {
                let terminal_label = if let Some(terminal) = self.terminal.as_ref() {
                    if let Some(error) = terminal.error() {
                        format!("{} · {error}", self.terminal_workspace.display())
                    } else if let Some(status) = terminal.exit_status() {
                        format!("{} · {status}", self.terminal_workspace.display())
                    } else {
                        format!(
                            "{} · PID {}",
                            self.terminal_workspace.display(),
                            terminal
                                .process_id()
                                .map(|pid| pid.to_string())
                                .unwrap_or_else(|| "?".into())
                        )
                    }
                } else {
                    self.terminal_workspace.display().to_string()
                };
                let history = self
                    .terminal
                    .as_ref()
                    .map(|pane| pane.history_offset())
                    .unwrap_or(0);
                let restart = self
                    .terminal
                    .as_ref()
                    .is_none_or(|pane| pane.exit_status().is_some() || pane.error().is_some());
                let label = if restart {
                    format!(
                        "[{}] {}",
                        i18n::tr("right.restart", "重新启动"),
                        terminal_label
                    )
                } else if history > 0 {
                    format!(
                        "{} {} · End · {}",
                        i18n::tr("right.history", "历史"),
                        history,
                        terminal_label
                    )
                } else {
                    terminal_label
                };
                if restart {
                    hits.insert(
                        0,
                        (Rect::new(body.x, body.y, body.width, 1), Action::Restart),
                    );
                }
                frame.render_widget(
                    Paragraph::new(label).style(theme.fg(theme.text_dim)),
                    Rect::new(body.x, body.y, body.width, 1),
                );
                let content = Rect::new(
                    body.x,
                    body.y + 1,
                    body.width,
                    body.height.saturating_sub(1),
                );
                if let Some(terminal) = self.terminal.as_ref() {
                    let screen = terminal.screen();
                    for row in 0..content.height {
                        for col in 0..content.width {
                            if let Some(cell) = screen.cell(row, col) {
                                if cell.is_wide_continuation() {
                                    continue;
                                }
                                let width = if cell.is_wide() { 2 } else { 1 };
                                if width > content.width - col {
                                    continue;
                                }
                                let rect = Rect::new(content.x + col, content.y + row, width, 1);
                                let style = terminal_style(cell, &theme);
                                frame.render_widget(
                                    Paragraph::new(cell.contents()).style(style),
                                    rect,
                                );
                            }
                        }
                    }
                    if self.focused && terminal.history_offset() == 0 && !screen.hide_cursor() {
                        let (row, col) = screen.cursor_position();
                        if row < content.height && col < content.width {
                            frame.set_cursor_position((content.x + col, content.y + row));
                        }
                    }
                } else {
                    frame.render_widget(
                        Paragraph::new(self.terminal_error.as_str()).wrap(Wrap { trim: false }),
                        content,
                    );
                }
            }
            Tab::Chat => {
                if let Some(chat) = &self.chat {
                    self.controls.borrow_mut().clear();
                    let row = Rect::new(body.x, body.bottom().saturating_sub(2), body.width, 1);
                    let route_label = chat
                        .route
                        .as_ref()
                        .map(|(m, p)| format!("{p}/{m}"))
                        .unwrap_or_else(|| format!("{}/{}", model.provider, model.model));
                    let permission_label =
                        i18n::permission_name(chat.permission.unwrap_or(model.permission_mode));
                    let labels = [
                        (Control::Permission, permission_label),
                        (
                            Control::Context,
                            i18n::tr("dialog.context_usage", "上下文用量"),
                        ),
                        (Control::Provider, route_label),
                    ];
                    let fixed = [row.width.min(12), row.width.saturating_sub(12).min(8)];
                    let mut x = row.x;
                    for (index, (control, label)) in labels.into_iter().enumerate() {
                        let width = if index < 2 {
                            fixed[index]
                        } else {
                            row.right().saturating_sub(x)
                        };
                        if width == 0 {
                            continue;
                        }
                        let rect = Rect::new(x, row.y, width, 1);
                        let text = if control == Control::Context {
                            i18n::tr("right.context", "上下文")
                        } else {
                            label
                        };
                        let style = if layout::hovered(rect, model.pointer) {
                            theme.fg(theme.rose).bg(theme.bg_select)
                        } else {
                            theme.fg(theme.sage)
                        };
                        frame.render_widget(
                            Paragraph::new(layout::clip(&text, width.saturating_sub(1) as usize))
                                .style(style),
                            rect,
                        );
                        self.controls.borrow_mut().push((rect, control));
                        x += width;
                    }
                    let route = chat
                        .route
                        .as_ref()
                        .map(|(m, p)| format!("{p}/{m}"))
                        .unwrap_or_else(|| i18n::tr("right.route_pending", "首次发送时继承配置"));
                    let header = format!(
                        "{}
{}
{} · {} {} · {}",
                        chat.title(),
                        chat.workspace.display(),
                        chat.status,
                        i18n::tr("right.queued", "排队"),
                        chat.queue.len(),
                        route
                    );
                    frame.render_widget(
                        Paragraph::new(header).style(theme.fg(theme.text_dim)),
                        Rect::new(body.x, body.y, body.width, 3.min(body.height)),
                    );
                    let (lines, _) = chat::draft_lines(
                        &chat.draft,
                        chat.cursor,
                        body.width.saturating_sub(3).max(1) as usize,
                    );
                    let height = (lines.len() as u16)
                        .clamp(1, 5)
                        .min(body.height.saturating_sub(6).max(1));
                    let draft = Rect::new(
                        body.x,
                        body.bottom().saturating_sub(2 + height),
                        body.width,
                        height,
                    );
                    let mut slash_hits = crate::slash::matches(&chat.draft);
                    slash_hits.retain(|command| {
                        matches!(
                            command.name,
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
                        )
                    });
                    let slash_open = crate::slash::is_open(&chat.draft)
                        && !chat.slash_dismissed
                        && chat.paste_preview.is_none();
                    let slash_height = if slash_open {
                        (slash_hits.len().clamp(1, crate::slash::MAX_VISIBLE))
                            .min(draft.y.saturating_sub(body.y + 3) as usize)
                            as u16
                    } else {
                        0
                    };
                    let paste_height = if !slash_open
                        && chat.mentions.height() == 0
                        && chat.paste_preview.is_some()
                    {
                        chat.paste_preview
                            .as_deref()
                            .map(str::lines)
                            .map(Iterator::count)
                            .unwrap_or(1)
                            .clamp(1, 6) as u16
                    } else {
                        0
                    };
                    let notice_height = if !slash_open
                        && chat.mentions.height() == 0
                        && paste_height == 0
                        && chat.slash_notice.is_some()
                    {
                        chat.slash_notice
                            .as_deref()
                            .map(str::lines)
                            .map(Iterator::count)
                            .unwrap_or(1)
                            .clamp(1, 4) as u16
                    } else {
                        0
                    };
                    let overlay_height = paste_height.max(notice_height);
                    let mention_height = chat.mentions.height().min(
                        draft
                            .y
                            .saturating_sub(body.y + 3 + slash_height + overlay_height),
                    );
                    let mention_height = if slash_open { 0 } else { mention_height };
                    let mention_area = Rect::new(
                        body.x,
                        draft.y.saturating_sub(mention_height),
                        body.width,
                        mention_height,
                    );
                    if slash_height > 0 {
                        let menu = Rect::new(
                            body.x,
                            mention_area.y.saturating_sub(slash_height),
                            body.width,
                            slash_height,
                        );
                        for (row, command) in
                            slash_hits.iter().take(slash_height as usize).enumerate()
                        {
                            let selected = row == chat.slash_selected;
                            let label = format!("/{:<12} {}", command.name, command.about);
                            frame.render_widget(
                                Paragraph::new(crate::right_panel::layout::clip(
                                    &label,
                                    menu.width as usize,
                                ))
                                .style(if selected {
                                    theme.fg(theme.rose).bg(theme.bg_select)
                                } else {
                                    theme.fg(theme.text_dim)
                                }),
                                Rect::new(menu.x, menu.y + row as u16, menu.width, 1),
                            );
                        }
                    }
                    if overlay_height > 0 {
                        let overlay = Rect::new(
                            body.x,
                            mention_area.y.saturating_sub(overlay_height),
                            body.width,
                            overlay_height,
                        );
                        if let Some(preview) =
                            chat.paste_preview.as_deref().filter(|_| paste_height > 0)
                        {
                            crate::view::render_notice(frame, overlay, preview, &theme);
                        } else if let Some(notice) = chat.slash_notice.as_deref() {
                            crate::view::render_notice(frame, overlay, notice, &theme);
                        }
                    }
                    let content = Rect::new(
                        body.x,
                        body.y + 3,
                        body.width,
                        mention_area.y.saturating_sub(body.y + 3),
                    );
                    let mut side_model = model.clone();
                    side_model.projection = Some(&chat.projection);
                    side_model.input = &chat.draft;
                    side_model.running = chat.running;
                    side_model.search = None;
                    side_model.text_selection = None;
                    let mut cache = chat.cache.borrow_mut();
                    cache.ensure(
                        &chat.projection,
                        &chat.id.to_string(),
                        None,
                        model.hide_tools,
                        model.message_roles,
                        content.width.saturating_sub(2).max(8) as usize,
                        model.user_label,
                        &theme,
                    );
                    let count = cache.lines.len() + usize::from(cache.live_visible && chat.running);
                    let distance = chat.viewport(count, content.height as usize);
                    side_model.scroll = distance;
                    *chat.tool_rows.borrow_mut() = crate::conversation::render_transcript(
                        frame,
                        content,
                        &side_model,
                        &theme,
                        &mut cache,
                    );
                    crate::conversation::render_mentions(
                        frame,
                        mention_area,
                        &chat.mentions,
                        &theme,
                    );
                    let (cursor_row, cursor_col) = crate::conversation::render_editor(
                        frame,
                        draft,
                        &side_model,
                        &theme,
                        Some(chat.cursor),
                    );
                    if self.focused && cursor_col + 2 < draft.width && draft.height > 0 {
                        frame.set_cursor_position((draft.x + 2 + cursor_col, draft.y + cursor_row));
                    }
                    let controls = Rect::new(body.x, body.bottom() - 1, body.width, 1);
                    let send = Rect::new(controls.x, controls.y, controls.width.min(8), 1);
                    let cancel = Rect::new(
                        send.right(),
                        controls.y,
                        controls.width.saturating_sub(send.width).min(8),
                        1,
                    );
                    let bottom = Rect::new(
                        cancel.right(),
                        controls.y,
                        controls.right().saturating_sub(cancel.right()),
                        1,
                    );
                    for (rect, label, action) in [
                        (send, i18n::tr("right.send", "发送"), Action::Send),
                        (cancel, i18n::tr("right.cancel", "取消"), Action::Cancel),
                        (
                            bottom,
                            if distance > 0 {
                                format!(
                                    "{} +{}",
                                    i18n::tr("right.bottom", "到底部"),
                                    chat.unread.get()
                                )
                            } else {
                                String::new()
                            },
                            Action::Bottom,
                        ),
                    ] {
                        frame.render_widget(
                            Paragraph::new(label).style(theme.fg(theme.rose).bg(
                                if layout::hovered(rect, model.pointer) {
                                    theme.hairline
                                } else {
                                    theme.bg
                                },
                            )),
                            rect,
                        );
                        hits.insert(0, (rect, action));
                    }
                } else {
                    frame.render_widget(Paragraph::new(self.chat_error.as_str()), body);
                }
            }
        }
    }
}
#[cfg(test)]
mod tests;
