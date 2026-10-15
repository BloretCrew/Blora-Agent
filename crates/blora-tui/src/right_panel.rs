// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Independent right-hand tools and their project/session ownership.

mod chat;
mod editor;
mod files;
mod highlight;
mod layout;
use files::*;

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
    style::Style,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{
    collections::VecDeque,
    fs,
    io::Read,
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
enum Action {
    Tab(Tab),
    Close(Tab),
    Collapse,
    Restore,
    File(usize),
    FileSave,
    FileConfirm(bool),
    FileCancel,
    Back,
    Focus,
    Refresh,
    Restart,
    Send,
    Cancel,
    Bottom,
    ExpandId,
}

pub struct SideChat {
    pub id: SessionId,
    pub workspace: PathBuf,
    pub projection: SessionProjection,
    pub cancel: CancelToken,
    pub running: bool,
    draft: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    saved_draft: String,
    rendered: std::cell::Cell<usize>,
    max_scroll: std::cell::Cell<usize>,
    unread: std::cell::Cell<usize>,
    queue: VecDeque<String>,
    scroll: std::cell::Cell<usize>,
    pub status: String,
    route: Option<(String, String)>,
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
    tabs: Vec<Tab>,
    active: Tab,
    pub focused: bool,
    collapsed: bool,
    area: Option<Rect>,
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
    done_tx: Sender<(SessionId, String)>,
    done_rx: Receiver<(SessionId, String)>,
    cancellations: std::sync::Arc<std::sync::Mutex<Vec<CancelToken>>>,
}
impl Default for RightPanel {
    fn default() -> Self {
        let (done_tx, done_rx) = mpsc::channel();
        Self {
            tabs: vec![Tab::Info],
            active: Tab::Info,
            focused: false,
            collapsed: false,
            area: None,
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
            done_tx,
            done_rx,
            cancellations: Default::default(),
        }
    }
}
pub fn regions(area: Rect, collapsed: bool) -> (Rect, Option<Rect>) {
    if !collapsed && area.width >= 124 && area.height >= 18 {
        (
            Rect::new(area.x, area.y, area.width - 36, area.height),
            Some(Rect::new(area.right() - 36, area.y, 36, area.height)),
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
    pub fn chat_focused(&self) -> bool {
        self.focused && self.active == Tab::Chat
    }
    pub fn chat_tool_rows(&self) -> Vec<(Rect, SessionId, Vec<usize>)> {
        if self.active != Tab::Chat || self.collapsed || self.area.is_none() {
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
        self.area = regions(area, self.collapsed).1;
        if self.area.is_none() {
            self.focused = false;
        }
        if self.tabs.contains(&Tab::Files) {
            self.files.sync(workspace);
            if matches!(self.files.pending_leave, Some(Leave::Project(_))) {
                self.active = Tab::Files;
                self.focused = true;
            }
        }
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.poll();
            if let Some(area) = self.area {
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
                    title: Some(i18n::tr("right.session_title", "侧边对话")),
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
                            history: vec![],
                            history_index: None,
                            saved_draft: String::new(),
                            rendered: 0.into(),
                            max_scroll: 0.into(),
                            unread: 0.into(),
                            queue: VecDeque::new(),
                            scroll: 0.into(),
                            status: String::new(),
                            route: None,
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
            options: crate::tui_run_options(permission, model, provider, token),
        })
    }
    pub fn event(&mut self, event: &Event, runtime: &Runtime, workspace: &Path) -> bool {
        if let Event::Mouse(mouse) = event {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
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
                            self.files.open(index);
                        }
                        Action::FileSave => {
                            self.files.save();
                        }
                        Action::FileConfirm(save) => self.confirm_file_leave(save),
                        Action::FileCancel => self.files.cancel_leave(),
                        Action::Back => self.files.back(),
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
                .is_some_and(|rect| rect.contains((mouse.column, mouse.row).into()))
            {
                let delta = match mouse.kind {
                    MouseEventKind::ScrollDown => 3isize,
                    MouseEventKind::ScrollUp => -3,
                    _ => return true,
                };
                match self.active {
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
                            chat.scroll.set(
                                chat.scroll
                                    .get()
                                    .saturating_add_signed(-delta)
                                    .min(chat.max_scroll.get()),
                            );
                        }
                    }
                    Tab::Terminal => {
                        if let Some(terminal) = self.terminal.as_mut() {
                            let content = self.area.map(|area| {
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
        if !self.focused || self.area.is_none() {
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
        if let Event::Key(key) = event {
            if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
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
                }
                if let Some(editor) = self.files.editor.as_mut() {
                    match event {
                        Event::Paste(text) => editor.insert(text),
                        Event::Key(key) => editor.key(*key),
                        _ => {}
                    }
                    self.files.preview = Some(editor.text.clone());
                    let (row, _) = editor.position();
                    let page = self
                        .area
                        .map_or(10, |r| r.height.saturating_sub(7) as usize)
                        .max(1);
                    if row < self.files.scroll {
                        self.files.scroll = row;
                    }
                    if row >= self.files.scroll + page {
                        self.files.scroll = row + 1 - page;
                    }
                    let column = unicode_width::UnicodeWidthStr::width(
                        editor.text[..editor.cursor]
                            .rsplit('\n')
                            .next()
                            .unwrap_or("")
                            .replace('\t', "    ")
                            .as_str(),
                    );
                    let width = self
                        .area
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
                    match event {
                        Event::Paste(text) => chat.insert(text),
                        Event::Key(key) if key.code == KeyCode::PageUp => chat
                            .scroll
                            .set((chat.scroll.get() + 10).min(chat.max_scroll.get())),
                        Event::Key(key) if key.code == KeyCode::PageDown => {
                            chat.scroll.set(chat.scroll.get().saturating_sub(10))
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
        }
    }
    pub fn draw(&self, frame: &mut Frame<'_>, model: &FrameModel<'_>, canvas: Rect) {
        let theme = Theme::current();
        let mut hits = self.hits.borrow_mut();
        hits.clear();
        let restore = Rect::new(canvas.right().saturating_sub(4), canvas.y, 4, 1);
        if self.area.is_none() {
            frame.render_widget(
                Paragraph::new("[◧]").style(theme.fg(theme.text_dim)),
                restore,
            );
            hits.push((restore, Action::Restore));
            return;
        }
        let area = self.area.unwrap();
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(theme.fg(theme.hairline))
                .style(theme.base()),
            area,
        );
        let inner = Rect::new(area.x + 1, area.y, area.width - 1, area.height);
        for (tab, rect) in self.tabs.iter().zip(layout::tabs(inner, self.tabs.len())) {
            let selected = *tab == self.active;
            let style = if selected || layout::hovered(rect, model.pointer) {
                theme
                    .fg(theme.rose)
                    .bg(theme.hairline)
                    .add_modifier(ratatui::style::Modifier::UNDERLINED)
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
                    &tab.label(),
                    rect.width.saturating_sub(reserve) as usize,
                ))
                .style(style),
                rect,
            );
            hits.push((rect, Action::Tab(*tab)));
            if *tab != Tab::Info && rect.width >= 2 {
                let close = Rect::new(rect.right() - 1, rect.y, 1, 1);
                frame.render_widget(Paragraph::new("×").style(style), close);
                hits.insert(0, (close, Action::Close(*tab)));
            }
        }
        let collapse = Rect::new(inner.right() - 2, inner.y, 2, 1);
        frame.render_widget(Paragraph::new("›"), collapse);
        hits.insert(0, (collapse, Action::Collapse));
        let hint = if self.focused {
            i18n::tr("right.focused", "● 焦点 · Ctrl+G 返回 · Ctrl+W 关闭")
        } else {
            i18n::tr("right.focus", "点击聚焦 · Ctrl+Q 退出")
        };
        frame.render_widget(
            Paragraph::new(hint).style(theme.fg(theme.text_mute)),
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        );
        let body = layout::body(inner);
        hits.push((body, Action::Focus));
        match self.active {
            Tab::Info => {
                let unknown = i18n::tr("right.unknown", "未知");
                let session = model.sessions.get(model.index);
                let p = model.projection;
                let harness = p.map(|p| p.harness).unwrap_or_default();
                let external = harness != blora_types::HarnessKind::Blora;
                let title = session
                    .and_then(|s| s.title.as_deref())
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
            Tab::Files => self.draw_files(frame, model, body, &mut hits),
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
                    let route = chat
                        .route
                        .as_ref()
                        .map(|(m, p)| format!("{p}/{m}"))
                        .unwrap_or_else(|| i18n::tr("right.route_pending", "首次发送时固定配置"));
                    let header = format!(
                        "{} · Blora
{}
{} · {} {} · {}",
                        layout::clip(&chat.id.to_string(), 8),
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
                        .min(body.height.saturating_sub(5).max(1));
                    let draft = Rect::new(
                        body.x,
                        body.bottom().saturating_sub(1 + height),
                        body.width,
                        height,
                    );
                    let content = Rect::new(
                        body.x,
                        body.y + 3,
                        body.width,
                        draft.y.saturating_sub(body.y + 3),
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
fn terminal_style(cell: &vt100::Cell, theme: &Theme) -> Style {
    fn color(color: vt100::Color, default: ratatui::style::Color) -> ratatui::style::Color {
        match color {
            vt100::Color::Default => default,
            vt100::Color::Idx(index) => ratatui::style::Color::Indexed(index),
            vt100::Color::Rgb(r, g, b) => ratatui::style::Color::Rgb(r, g, b),
        }
    }
    let mut style = Style::default()
        .fg(color(cell.fgcolor(), theme.text))
        .bg(color(cell.bgcolor(), theme.bg));
    if cell.bold() {
        style = style.add_modifier(ratatui::style::Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(ratatui::style::Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(ratatui::style::Modifier::REVERSED);
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thresholds_preserve_left_and_minimum_center() {
        for (width, height, visible) in [
            (124, 18, true),
            (123, 30, false),
            (160, 17, false),
            (160, 40, true),
        ] {
            let (canvas, right) = regions(Rect::new(0, 0, width, height), false);
            assert_eq!(right.is_some(), visible);
            if visible {
                assert!(canvas.width - 28 >= 60);
                assert_eq!(right.unwrap().width, 36);
            }
        }
        assert!(regions(Rect::new(0, 0, 160, 40), true).1.is_none());
    }
    #[test]
    fn closes_only_requested_tab_and_preserves_info() {
        let mut panel = RightPanel::default();
        panel.tabs.extend([Tab::Files, Tab::Chat]);
        panel.active = Tab::Files;
        panel.close(Tab::Files);
        assert_eq!(panel.tabs, vec![Tab::Info, Tab::Chat]);
        assert_eq!(panel.active, Tab::Info);
        panel.close(Tab::Info);
        assert!(panel.tabs.contains(&Tab::Info));
    }
    #[test]
    fn file_boundary_and_binary_preview() {
        let root = std::env::temp_dir().join(format!("blora-right-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("binary"), [0, 1, 2]).unwrap();
        assert!(read_file_data(&root, &root.join("binary"), true).is_err());
        assert!(read_file_data(&root, &root.join(".."), false).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stale_file_generations_are_discarded() {
        let mut files = Files::new();
        files.generation = 2;
        files.worker.work.lock().unwrap().result = Some(FileResult {
            generation: 1,
            result: Ok(FileData::Preview("stale".into())),
        });
        files.sync(Path::new(""));
        assert!(files.preview.is_none());
    }
    fn runtime() -> Runtime {
        Runtime::new(blora_storage::SqliteStore::open_in_memory().unwrap())
    }
    #[test]
    fn tabs_deduplicate_and_chat_keeps_original_project_and_history() {
        let runtime = runtime();
        let mut panel = RightPanel::default();
        panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
        let id = panel.chat.as_ref().unwrap().id.clone();
        panel.open(Tab::Chat, &runtime, Path::new("/other"));
        assert_eq!(
            panel.tabs.iter().filter(|tab| **tab == Tab::Chat).count(),
            1
        );
        assert_eq!(panel.chat.as_ref().unwrap().workspace, Path::new("/tmp"));
        assert_eq!(panel.chat.as_ref().unwrap().id, id);
        panel.close(Tab::Chat);
        assert!(runtime.show_session(&id).is_ok());
        panel.open(Tab::Chat, &runtime, Path::new("/other"));
        assert_ne!(panel.chat.as_ref().unwrap().id, id);
    }
    #[test]
    fn focus_escape_hide_paste_and_quit_routing() {
        use crossterm::event::KeyEvent;
        let runtime = runtime();
        let mut panel = RightPanel::default();
        panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
        panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
        assert!(panel.event(
            &Event::Paste("side draft".into()),
            &runtime,
            Path::new("/tmp")
        ));
        assert_eq!(panel.chat.as_ref().unwrap().draft, "side draft");
        assert!(!panel.event(
            &Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            &runtime,
            Path::new("/tmp")
        ));
        assert!(panel.event(
            &Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)),
            &runtime,
            Path::new("/tmp")
        ));
        assert!(!panel.focused);
        panel.focused = true;
        panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 123, 18));
        assert!(!panel.focused);
        assert_eq!(panel.chat.as_ref().unwrap().draft, "side draft");
    }
    #[test]
    fn request_permissions_login_queue_and_completion_are_session_scoped() {
        let runtime = runtime();
        let mut panel = RightPanel::default();
        panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
        panel.chat.as_mut().unwrap().queue.push_back("first".into());
        assert!(
            panel
                .request(blora_policy::PermissionMode::Ask, "model", "blora", None)
                .is_none()
        );
        assert!(!panel.chat.as_ref().unwrap().running);
        assert_eq!(panel.chat.as_ref().unwrap().queue.front().unwrap(), "first");
        panel.chat.as_mut().unwrap().route = None;
        panel
            .chat
            .as_mut()
            .unwrap()
            .queue
            .push_back("second".into());
        let request = panel
            .request(blora_policy::PermissionMode::Plan, "model", "mock", None)
            .unwrap();
        assert_eq!(
            request.options.permission,
            Some(blora_policy::PermissionMode::Plan)
        );
        assert!(request.options.can_ask);
        assert!(request.options.interactive);
        assert!(
            panel
                .request(blora_policy::PermissionMode::Ask, "model", "mock", None)
                .is_none()
        );
        panel
            .done_tx
            .send((SessionId::generate(), "foreign".into()))
            .unwrap();
        panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
        assert!(panel.chat.as_ref().unwrap().running);
        panel
            .done_tx
            .send((request.id.clone(), "done".into()))
            .unwrap();
        panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
        let next = panel
            .request(blora_policy::PermissionMode::Ask, "model", "mock", None)
            .unwrap();
        assert_eq!(next.prompt, "second");
        panel.close(Tab::Chat);
        assert!(next.cancel.is_cancelled());
    }
    #[test]
    fn guard_cancels_side_without_touching_main_token() {
        let runtime = runtime();
        let mut panel = RightPanel::default();
        let main = CancelToken::new();
        panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
        panel
            .chat
            .as_mut()
            .unwrap()
            .queue
            .push_back("prompt".into());
        let request = panel
            .request(blora_policy::PermissionMode::Ask, "model", "mock", None)
            .unwrap();
        drop(panel.cancel_guard());
        assert!(request.cancel.is_cancelled());
        assert!(!main.is_cancelled());
    }
    #[test]
    fn mock_runs_have_separate_canonical_transcripts() {
        let runtime = runtime();
        let main = runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: "/tmp".into(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        let mut panel = RightPanel::default();
        panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
        let side = panel.chat.as_ref().unwrap().id.clone();
        let options =
            crate::tui_run_options(blora_policy::PermissionMode::Ask, "mock", "mock", None);
        thread::scope(|scope| {
            let runtime = &runtime;
            let options = &options;
            let main = &main;
            let side = &side;
            let a =
                scope.spawn(move || runtime.run(main, "main-only", &CancelToken::new(), options));
            let b =
                scope.spawn(move || runtime.run(side, "side-only", &CancelToken::new(), options));
            a.join().unwrap().unwrap();
            b.join().unwrap().unwrap();
        });
        for (id, expected, foreign) in [
            (&main, "main-only", "side-only"),
            (&side, "side-only", "main-only"),
        ] {
            let projection = runtime.show_session(id).unwrap();
            assert!(
                projection
                    .transcript
                    .iter()
                    .any(|item| matches!(item,TranscriptItem::User{text,..} if text==expected))
            );
            assert!(
                !projection
                    .transcript
                    .iter()
                    .any(|item| matches!(item,TranscriptItem::User{text,..} if text==foreign))
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn symlink_outside_project_is_not_listed_or_previewed() {
        let root = std::env::temp_dir().join(format!("blora-right-link-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(root.parent().unwrap(), root.join("outside")).unwrap();
        let FileData::List(entries) = read_file_data(&root, &root, false).unwrap() else {
            panic!("expected list")
        };
        assert!(entries.is_empty());
        assert!(read_file_data(&root, &root.join("outside"), false).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn preview_is_bounded_and_empty_files_are_explicit() {
        let root = std::env::temp_dir().join(format!("blora-right-size-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("large"), vec![b'x'; 70000]).unwrap();
        fs::write(root.join("empty"), []).unwrap();
        let FileData::Preview(text) = read_file_data(&root, &root.join("large"), true).unwrap()
        else {
            panic!("expected preview")
        };
        assert!(text.len() < 66000);
        assert!(text.starts_with(&"x".repeat(65536)));
        let FileData::Preview(text) = read_file_data(&root, &root.join("empty"), true).unwrap()
        else {
            panic!("expected preview")
        };
        assert!(!text.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn question_targets_stay_with_side_form_while_main_also_waits() {
        let runtime = runtime();
        let mut panel = RightPanel::default();
        panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
        let side_pending = blora_session::PendingInput {
            request_id: "side-request".into(),
            question: "side?".into(),
            options: vec![],
            questions: vec![],
        };
        panel.chat.as_mut().unwrap().projection.pending_input = Some(side_pending.clone());
        let form = crate::question::QuestionForm::open(&side_pending, Path::new("/tmp"));
        let mut main = SessionProjection::new();
        main.pending_input = Some(blora_session::PendingInput {
            request_id: "main-request".into(),
            question: "main?".into(),
            options: vec![],
            questions: vec![],
        });
        let main_id = SessionId::generate();
        let (side, id, path, pending) = panel.question_target(
            Some(&main),
            Some(&form),
            Some(&main_id),
            Path::new("/other"),
        );
        assert!(side);
        assert_eq!(id.as_ref(), panel.chat.as_ref().map(|chat| &chat.id));
        assert_eq!(path, Path::new("/tmp"));
        assert_eq!(pending.unwrap().request_id, "side-request");
        let (side, id, _, pending) =
            panel.question_target(Some(&main), None, Some(&main_id), Path::new("/other"));
        assert!(!side);
        assert_eq!(id, Some(main_id));
        assert_eq!(pending.unwrap().request_id, "main-request");
    }
    #[test]
    fn file_worker_keeps_one_latest_pending_request() {
        let mut files = Files::new();
        {
            let mut work = files.worker.work.lock().unwrap();
            work.stop = true;
        }
        files.worker.ready.notify_one();
        files.handle.take().unwrap().join().unwrap();
        files.worker.work.lock().unwrap().stop = false;
        let (release, gate) = mpsc::channel();
        struct ReleaseOnDrop(Option<Sender<()>>);
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }
        let mut release = ReleaseOnDrop(Some(release));
        let worker = files.worker.clone();
        files.handle = Some(thread::spawn(move || {
            gate.recv().unwrap();
            worker.run();
        }));
        let worker_id = files.handle.as_ref().unwrap().thread().id();
        files.root = PathBuf::from("/tmp");
        files.load(PathBuf::from("/tmp/first"), true);
        let first_cancel = files.cancel.clone();
        for index in 0..100 {
            files.load(PathBuf::from(format!("/tmp/latest-{index}")), true);
        }
        assert!(first_cancel.is_cancelled());
        assert_eq!(files.handle.as_ref().unwrap().thread().id(), worker_id);
        {
            let work = files.worker.work.lock().unwrap();
            let pending = work.pending.as_ref().unwrap();
            assert_eq!(pending.generation, files.generation);
            assert_eq!(pending.path, Path::new("/tmp/latest-99"));
            assert!(work.result.is_none());
        }
        files.close();
        assert!(files.cancel.is_cancelled());
        assert!(files.worker.work.lock().unwrap().pending.is_none());
        release.0.take().unwrap().send(()).unwrap();
    }
    #[test]
    fn file_worker_drop_stops_and_joins_thread() {
        let files = Files::new();
        let worker = std::sync::Arc::downgrade(&files.worker);
        let cancel = files.cancel.clone();
        drop(files);
        assert!(cancel.is_cancelled());
        assert!(worker.upgrade().is_none());
    }
    #[test]
    fn file_cancel_prevents_enumeration_and_preview_io() {
        let cancel = CancelToken::new();
        cancel.cancel();
        for preview in [false, true] {
            let result = read_file_data_cancelled(
                Path::new("/missing-project"),
                Path::new("/missing-file"),
                preview,
                &cancel,
            );
            assert!(matches!(result, Err(error) if error == "file request cancelled"));
        }
    }
    #[test]
    fn file_worker_publishes_latest_result_and_close_cancels_active_work() {
        let root = std::env::temp_dir().join(format!("blora-right-worker-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("last"), "latest preview").unwrap();
        let mut files = Files::new();
        files.root = root.clone();
        for _ in 0..100 {
            files.load(root.clone(), false);
        }
        files.load(root.join("last"), true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while files.preview.is_none() && std::time::Instant::now() < deadline {
            files.sync(&root);
            thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(files.preview.as_deref(), Some("latest preview"));
        files.load(root.clone(), false);
        let active_cancel = files.cancel.clone();
        files.close();
        assert!(active_cancel.is_cancelled());
        assert!(files.worker.work.lock().unwrap().pending.is_none());
        assert!(files.worker.work.lock().unwrap().result.is_none());
        drop(files);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn four_tabs_are_disjoint_clipped_and_leave_collapse_space() {
        for width in [4, 12, 24, 35, 60] {
            let inner = Rect::new(9, 2, width, 18);
            let rects = layout::tabs(inner, 4);
            assert_eq!(rects.len(), 4);
            for (index, rect) in rects.iter().enumerate() {
                assert!(rect.right() <= inner.right() - 2);
                if index > 0 {
                    assert_eq!(rects[index - 1].right(), rect.x);
                }
                for x in rect.x..rect.right() {
                    assert_eq!(
                        rects.iter().filter(|r| r.contains((x, 2).into())).count(),
                        1
                    );
                }
            }
        }
        assert_eq!(layout::clip("中文a", 3), "中");
        assert_eq!(layout::clip("中文a", 4), "中文");
        assert_eq!(layout::body(Rect::new(0, 0, 4, 2)).height, 0);
        assert!(layout::hovered(Rect::new(2, 3, 4, 1), Some((5, 3))));
        assert!(!layout::hovered(Rect::new(2, 3, 4, 1), Some((6, 3))));
    }
    #[test]
    fn side_editor_utf8_delete_multiline_history_and_cursor() {
        use crossterm::event::KeyEvent;
        let runtime = runtime();
        let mut panel = RightPanel::default();
        panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
        let chat = panel.chat.as_mut().unwrap();
        chat.insert("你🙂好");
        chat.edit(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        chat.edit(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(chat.draft, "你好");
        chat.edit(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        chat.insert("ab");
        chat.edit(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        chat.insert("第二行");
        let saved = chat.draft.clone();
        chat.send();
        assert_eq!(chat.queue.front(), Some(&saved));
        assert_eq!(chat.cursor, 0);
        chat.insert("未发送");
        chat.edit(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(chat.draft, saved);
        chat.edit(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(chat.draft, "未发送");
        assert!(chat.draft.is_char_boundary(chat.cursor));
        let (lines, cursor) = chat::draft_lines("中文a\n你", "中文a\n你".len(), 4);
        assert_eq!(lines, vec!["中文", "a", "你"]);
        assert_eq!(cursor, (2, 2));
        let (_, cursor) = chat::draft_lines("中文", "中文".len(), 4);
        assert_eq!(cursor, (1, 0));
    }
    #[test]
    fn side_scroll_direction_bounds_anchor_and_markdown_filter() {
        let runtime = runtime();
        let mut panel = RightPanel::default();
        panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
        panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
        let area = panel.area.unwrap();
        panel.chat.as_ref().unwrap().viewport(100, 10);
        let mouse = |kind| {
            Event::Mouse(crossterm::event::MouseEvent {
                kind,
                column: area.x + 2,
                row: area.y + 5,
                modifiers: KeyModifiers::NONE,
            })
        };
        panel.event(
            &mouse(MouseEventKind::ScrollUp),
            &runtime,
            Path::new("/tmp"),
        );
        let chat = panel.chat.as_ref().unwrap();
        assert_eq!(chat.scroll.get(), 3);
        assert_eq!(chat.viewport(105, 10), 8);
        assert_eq!(chat.unread.get(), 5);
        assert_eq!(105 - 10 - chat.scroll.get(), 87);
        panel.event(
            &mouse(MouseEventKind::ScrollDown),
            &runtime,
            Path::new("/tmp"),
        );
        let chat = panel.chat.as_ref().unwrap();
        assert_eq!(chat.scroll.get(), 5);
        assert_eq!(chat.viewport(4, 10), 0);
        assert_eq!(chat.unread.get(), 0);
        panel.focused = true;
        panel.event(
            &Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2,
                row: 8,
                modifiers: KeyModifiers::NONE,
            }),
            &runtime,
            Path::new("/tmp"),
        );
        assert!(!panel.focused);
    }
    fn await_files(files: &mut Files, root: &Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            files.sync(root);
            if files.message != i18n::tr("right.loading", "加载中…") {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "file worker timed out"
            );
            thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[test]
    fn files_refresh_return_selection_and_deleted_preview_do_not_stale() {
        let root = std::env::temp_dir().join(format!("blora-right-refresh-{}", std::process::id()));
        fs::create_dir_all(root.join("folder")).unwrap();
        fs::write(root.join("a"), "old").unwrap();
        fs::write(root.join("b"), "last").unwrap();
        let mut files = Files::new();
        files.sync(&root);
        await_files(&mut files, &root);
        files.open(2);
        await_files(&mut files, &root);
        assert_eq!(files.preview.as_deref(), Some("last"));
        files.back();
        assert_eq!(files.selected, 2);
        files.open(0);
        await_files(&mut files, &root);
        files.back();
        await_files(&mut files, &root);
        assert_eq!(files.entries[files.selected].path, root.join("folder"));
        files.open(1);
        await_files(&mut files, &root);
        fs::write(root.join("a"), "new").unwrap();
        files.refresh();
        await_files(&mut files, &root);
        assert_eq!(files.preview.as_deref(), Some("new"));
        fs::remove_file(root.join("a")).unwrap();
        files.refresh();
        await_files(&mut files, &root);
        assert!(files.preview.is_none());
        assert!(!files.message.is_empty());
        files.back();
        await_files(&mut files, &root);
        assert!(files.entries.iter().all(|e| e.path != root.join("a")));
        drop(files);
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn terminal_panel_history_keys_and_restart_only_after_exit() {
        use crossterm::event::KeyEvent;
        let runtime = runtime();
        let mut panel = RightPanel::default();
        panel.open(Tab::Terminal, &runtime, Path::new("/tmp"));
        panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
        let pid = panel.terminal.as_ref().unwrap().process_id();
        panel.restart_terminal();
        assert_eq!(panel.terminal.as_ref().unwrap().process_id(), pid);
        panel.terminal.as_mut().unwrap().input(b"stty -echo; i=0; while [ $i -lt 70 ]; do echo HISTORY_$i; i=$((i+1)); done; printf 'HISTORY_%s\\n' READY\r").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let pane = panel.terminal.as_mut().unwrap();
            pane.poll();
            if pane.screen().contents().contains("HISTORY_READY") {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(std::time::Duration::from_millis(10));
        }
        panel.event(
            &Event::Key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::SHIFT)),
            &runtime,
            Path::new("/tmp"),
        );
        assert!(panel.terminal.as_ref().unwrap().history_offset() > 0);
        panel.event(
            &Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            &runtime,
            Path::new("/tmp"),
        );
        assert_eq!(panel.terminal.as_ref().unwrap().history_offset(), 0);
        panel.terminal.as_mut().unwrap().input(b"exit\r").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while panel.terminal.as_ref().unwrap().exit_status().is_none() {
            panel.terminal.as_mut().unwrap().poll();
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(std::time::Duration::from_millis(10));
        }
        panel.restart_terminal();
        assert!(panel.terminal.as_ref().unwrap().exit_status().is_none());
        assert_ne!(panel.terminal.as_ref().unwrap().process_id(), pid);
    }
    #[test]
    fn drawn_chat_cursor_tabs_controls_and_info_expand_hit_regions() {
        use ratatui::{Terminal, backend::TestBackend};
        let runtime = runtime();
        let mut panel = RightPanel::default();
        panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
        panel.tabs = vec![Tab::Info, Tab::Files, Tab::Terminal, Tab::Chat];
        panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
        panel.chat.as_mut().unwrap().insert("中文\n草稿");
        let mut terminal = Terminal::new(TestBackend::new(124, 18)).unwrap();
        let model = FrameModel {
            dialog_stack: &[],
            workspace: Path::new("/tmp"),
            sessions: &[],
            index: 0,
            sidebar: &crate::view::SidebarState::default(),
            right_panel: Some(&panel),
            projection: None,
            pending: &[],
            input: "",
            status: "ready",
            notice: None,
            paste_preview: None,
            passport_dialog: None,
            provider_dialog: None,
            add_provider_dialog: None,
            theme_dialog: None,
            context_dialog: None,
            settings_dialog: None,
            harness_dialog: None,
            permission_dialog: None,
            tool_dialog: None,
            tool_detail_dialog: None,
            slash_hits: &[],
            slash_selected: 0,
            search: None,
            hide_tools: false,
            message_roles: true,
            scroll: 0,
            permission_mode: blora_policy::PermissionMode::Ask,
            model: "mock",
            provider: "mock",
            mode_menu: None,
            project_picker: None,
            folder_picker: None,
            question_dialog: None,
            session_picker: None,
            git_dialog: None,
            git_status: None,
            user_label: "you",
            running: true,
            tick: 0,
            pointer: None,
            text_selection: None,
        };
        terminal
            .draw(|frame| panel.draw(frame, &model, Rect::new(0, 0, 88, 18)))
            .unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        assert!(cursor.x >= 89 && cursor.x < 124);
        assert!(cursor.y >= 13 && cursor.y < 17);
        let hits = panel.hits.borrow();
        assert_eq!(
            hits.iter()
                .filter(|(_, a)| matches!(a, Action::Close(_)))
                .count(),
            3
        );
        assert!(hits.iter().any(|(_, a)| matches!(a, Action::Send)));
        assert!(hits.iter().any(|(_, a)| matches!(a, Action::Cancel)));
        for (rect, _) in hits.iter() {
            assert!(rect.right() <= 124);
            assert!(rect.bottom() <= 18);
        }
        drop(hits);
        drop(model);
        panel.active = Tab::Info;
        let model = FrameModel {
            dialog_stack: &[],
            workspace: Path::new("/tmp"),
            sessions: &[],
            index: 0,
            sidebar: &crate::view::SidebarState::default(),
            right_panel: Some(&panel),
            projection: None,
            pending: &[],
            input: "",
            status: "ready",
            notice: None,
            paste_preview: None,
            passport_dialog: None,
            provider_dialog: None,
            add_provider_dialog: None,
            theme_dialog: None,
            context_dialog: None,
            settings_dialog: None,
            harness_dialog: None,
            permission_dialog: None,
            tool_dialog: None,
            tool_detail_dialog: None,
            slash_hits: &[],
            slash_selected: 0,
            search: None,
            hide_tools: false,
            message_roles: true,
            scroll: 0,
            permission_mode: blora_policy::PermissionMode::Ask,
            model: "mock",
            provider: "mock",
            mode_menu: None,
            project_picker: None,
            folder_picker: None,
            question_dialog: None,
            session_picker: None,
            git_dialog: None,
            git_status: None,
            user_label: "you",
            running: true,
            tick: 0,
            pointer: None,
            text_selection: None,
        };
        terminal
            .draw(|frame| panel.draw(frame, &model, Rect::new(0, 0, 88, 18)))
            .unwrap();
        assert!(
            panel
                .hits
                .borrow()
                .iter()
                .any(|(_, a)| matches!(a, Action::ExpandId))
        );
        assert_eq!(
            panel
                .hits
                .borrow()
                .iter()
                .filter(|(_, a)| matches!(a, Action::Tab(Tab::Files | Tab::Terminal | Tab::Chat)))
                .count(),
            6
        );
    }
    #[test]
    fn side_markdown_and_system_events_follow_main_transcript() {
        let runtime = runtime();
        let mut panel = RightPanel::default();
        panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
        let chat = panel.chat.as_mut().unwrap();
        chat.projection.transcript.push(TranscriptItem::System {
            summary: "INTERNAL_ONLY".into(),
            event_id: blora_types::EventId::generate(),
        });
        chat.projection.transcript.push(TranscriptItem::Assistant {
            text: "# 标题\n\n```rust\nlet value = 1;\n```".into(),
            event_id: blora_types::EventId::generate(),
        });
        let text = chat
            .lines(35, &Theme::current())
            .into_iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("标题"));
        assert!(text.contains("let value = 1;"));
        assert!(text.contains("INTERNAL_ONLY"));
    }
}
