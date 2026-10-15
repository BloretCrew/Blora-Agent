// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Independent right-hand tools and their project/session ownership.

use crate::{i18n, theme::Theme, view::FrameModel};
use blora_runtime::{CancelToken, RunOptions, Runtime};
use blora_session::{SessionProjection, TranscriptItem};
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
    Back,
    Focus,
}
#[derive(Debug)]
struct FileEntry {
    path: PathBuf,
    directory: bool,
}
struct FileResult {
    generation: u64,
    result: Result<FileData, String>,
}
enum FileData {
    List(Vec<FileEntry>),
    Preview(String),
}
struct FileRequest {
    generation: u64,
    root: PathBuf,
    path: PathBuf,
    preview: bool,
    cancel: CancelToken,
}
#[derive(Default)]
struct FileWork {
    pending: Option<FileRequest>,
    result: Option<FileResult>,
    stop: bool,
}
#[derive(Default)]
struct FileWorker {
    work: std::sync::Mutex<FileWork>,
    ready: std::sync::Condvar,
}
impl FileWorker {
    fn run(&self) {
        loop {
            let request = {
                let mut work = self.work.lock().unwrap();
                while work.pending.is_none() && !work.stop {
                    work = self.ready.wait(work).unwrap();
                }
                if work.stop {
                    return;
                }
                work.pending.take().unwrap()
            };
            let result = read_file_data_cancelled(
                &request.root,
                &request.path,
                request.preview,
                &request.cancel,
            );
            let mut work = self.work.lock().unwrap();
            if !work.stop && !request.cancel.is_cancelled() {
                work.result = Some(FileResult {
                    generation: request.generation,
                    result,
                });
            }
        }
    }
}
struct Files {
    root: PathBuf,
    directory: PathBuf,
    generation: u64,
    entries: Vec<FileEntry>,
    selected: usize,
    preview: Option<String>,
    message: String,
    worker: std::sync::Arc<FileWorker>,
    handle: Option<thread::JoinHandle<()>>,
    cancel: CancelToken,
    scroll: usize,
}
impl Drop for Files {
    fn drop(&mut self) {
        self.cancel();
        {
            let mut work = self.worker.work.lock().unwrap();
            work.stop = true;
        }
        self.worker.ready.notify_one();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl Files {
    fn new() -> Self {
        let worker = std::sync::Arc::new(FileWorker::default());
        let background = worker.clone();
        let handle = thread::Builder::new()
            .name("blora-right-files".into())
            .spawn(move || background.run())
            .ok();
        Self {
            root: PathBuf::new(),
            directory: PathBuf::new(),
            generation: 0,
            entries: vec![],
            selected: 0,
            preview: None,
            message: String::new(),
            worker,
            handle,
            cancel: CancelToken::new(),
            scroll: 0,
        }
    }
    fn load(&mut self, path: PathBuf, preview: bool) {
        self.cancel();
        self.message = i18n::tr("right.loading", "加载中…");
        self.preview = None;
        self.scroll = 0;
        if !preview {
            self.selected = 0;
            self.directory = path.clone();
            self.entries.clear();
        }
        if self.handle.is_none() {
            self.message = "could not start file worker".to_owned();
            return;
        }
        self.cancel = CancelToken::new();
        let mut work = self.worker.work.lock().unwrap();
        work.pending = Some(FileRequest {
            generation: self.generation,
            root: self.root.clone(),
            path,
            preview,
            cancel: self.cancel.clone(),
        });
        self.worker.ready.notify_one();
    }
    fn cancel(&mut self) {
        self.cancel.cancel();
        self.generation = self.generation.wrapping_add(1);
        let mut work = self.worker.work.lock().unwrap();
        work.pending = None;
        work.result = None;
    }
    fn close(&mut self) {
        self.cancel();
        self.root.clear();
        self.directory.clear();
        self.entries.clear();
        self.preview = None;
        self.message.clear();
    }

    fn sync(&mut self, root: &Path) {
        if self.root != root {
            self.root = root.to_path_buf();
            self.load(root.to_path_buf(), false);
        }
        let update = self.worker.work.lock().unwrap().result.take();
        if let Some(update) = update {
            if update.generation != self.generation {
                return;
            }
            match update.result {
                Ok(FileData::List(entries)) => {
                    self.entries = entries;
                    self.message = if self.entries.is_empty() {
                        i18n::tr("right.empty", "空目录")
                    } else {
                        String::new()
                    };
                }
                Ok(FileData::Preview(text)) => {
                    self.preview = Some(text);
                    self.message = String::new();
                }
                Err(error) => self.message = error,
            }
        }
    }
    fn open(&mut self, index: usize) {
        if let Some(entry) = self.entries.get(index) {
            self.load(entry.path.clone(), !entry.directory);
        }
    }
    fn back(&mut self) {
        if !self.message.is_empty() {
            self.message.clear();
            self.cancel();
        }
        if self.preview.is_some() {
            self.preview = None;
            self.scroll = 0;
        } else if self.directory != self.root {
            if let Some(parent) = self.directory.parent() {
                self.load(parent.to_path_buf(), false);
            }
        }
    }
}
#[cfg(test)]
fn read_file_data(root: &Path, path: &Path, preview: bool) -> Result<FileData, String> {
    read_file_data_cancelled(root, path, preview, &CancelToken::new())
}
fn file_cancelled(cancel: &CancelToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("file request cancelled".to_owned())
    } else {
        Ok(())
    }
}
fn read_file_data_cancelled(
    root: &Path,
    path: &Path,
    preview: bool,
    cancel: &CancelToken,
) -> Result<FileData, String> {
    file_cancelled(cancel)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    file_cancelled(cancel)?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    file_cancelled(cancel)?;
    if !path.starts_with(&root) {
        return Err(i18n::tr("right.boundary", "路径超出项目边界"));
    }
    if preview {
        if !path.is_file() {
            return Err(i18n::tr("right.special", "仅支持预览普通文件"));
        }
        file_cancelled(cancel)?;
        let mut file = fs::File::open(&path).map_err(|e| e.to_string())?;
        let truncated = file.metadata().map_err(|e| e.to_string())?.len() > 65536;
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        while bytes.len() < 65536 {
            file_cancelled(cancel)?;
            let remaining = (65536 - bytes.len()).min(chunk.len());
            let count = file
                .read(&mut chunk[..remaining])
                .map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        file_cancelled(cancel)?;
        if bytes.contains(&0)
            || std::str::from_utf8(&bytes)
                .is_err_and(|error| error.error_len().is_some() || !truncated)
        {
            return Err(i18n::tr("right.binary", "二进制文件，无法预览"));
        }
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if truncated {
            text.push_str(&format!(
                "\n{}",
                i18n::tr("right.truncated", "仅显示前 64 KiB")
            ));
        }
        if text.is_empty() {
            text = i18n::tr("right.empty_file", "空文件");
        }
        Ok(FileData::Preview(text))
    } else {
        let mut entries = Vec::new();
        file_cancelled(cancel)?;
        let mut directory = fs::read_dir(&path).map_err(|e| e.to_string())?;
        for _ in 0..1001 {
            file_cancelled(cancel)?;
            let Some(entry) = directory.next() else {
                break;
            };
            file_cancelled(cancel)?;
            let entry = entry.map_err(|e| e.to_string())?;
            let canonical = match entry.path().canonicalize() {
                Ok(p) if p.starts_with(&root) => p,
                _ => continue,
            };
            file_cancelled(cancel)?;
            if !canonical.is_dir() && !canonical.is_file() {
                continue;
            }
            entries.push(FileEntry {
                directory: canonical.is_dir(),
                path: entry.path(),
            });
        }
        if entries.len() > 1000 {
            return Err(i18n::tr(
                "right.too_many",
                "目录超过 1000 项，请选择更小的目录",
            ));
        }
        file_cancelled(cancel)?;
        entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.path.cmp(&b.path)));
        file_cancelled(cancel)?;
        Ok(FileData::List(entries))
    }
}

pub struct SideChat {
    pub id: SessionId,
    pub workspace: PathBuf,
    pub projection: SessionProjection,
    pub cancel: CancelToken,
    pub running: bool,
    draft: String,
    queue: VecDeque<String>,
    scroll: usize,
    pub status: String,
    route: Option<(String, String)>,
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
    info_scroll: usize,
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
            info_scroll: 0,
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
                            queue: VecDeque::new(),
                            scroll: 0,
                            status: String::new(),
                            route: None,
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
    fn close(&mut self, tab: Tab) {
        if tab == Tab::Info {
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
                let hit = {
                    self.hits
                        .borrow()
                        .iter()
                        .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                        .cloned()
                };
                if let Some((_, action)) = hit {
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
                        Action::Back => self.files.back(),
                        Action::Focus => self.focused = true,
                    }
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
                    Tab::Info => self.info_scroll = self.info_scroll.saturating_add_signed(delta),
                    Tab::Files => {
                        if self.files.preview.is_some() {
                            self.files.scroll = self.files.scroll.saturating_add_signed(delta);
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
                            chat.scroll = chat.scroll.saturating_add_signed(delta);
                        }
                    }
                    _ => {}
                }
                return true;
            }
            return false;
        }
        if !self.focused || self.area.is_none() {
            return false;
        }
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Repeat && self.active == Tab::Terminal {
                if let Some(terminal) = self.terminal.as_mut() {
                    let _ = terminal.key(*key);
                }
                return true;
            }
            if key.kind != KeyEventKind::Press {
                return true;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('g') => {
                        self.focused = false;
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
                            let _ = terminal.paste(text);
                        }
                        Event::Key(key) => {
                            let _ = terminal.key(*key);
                        }
                        _ => {}
                    }
                }
            }
            Tab::Files => {
                if let Event::Key(key) = event {
                    match key.code {
                        KeyCode::Up => self.files.selected = self.files.selected.saturating_sub(1),
                        KeyCode::Down => {
                            self.files.selected = (self.files.selected + 1)
                                .min(self.files.entries.len().saturating_sub(1))
                        }
                        KeyCode::Enter => self.files.open(self.files.selected),
                        KeyCode::Backspace | KeyCode::Esc => self.files.back(),
                        KeyCode::PageDown => {
                            self.files.scroll = self.files.scroll.saturating_add(10)
                        }
                        KeyCode::PageUp => self.files.scroll = self.files.scroll.saturating_sub(10),
                        _ => {}
                    }
                }
            }
            Tab::Chat => {
                if let Some(chat) = self.chat.as_mut() {
                    match event {
                        Event::Paste(text) => chat.draft.push_str(text),
                        Event::Key(key) => match key.code {
                            KeyCode::Enter if !chat.draft.trim().is_empty() => {
                                chat.queue.push_back(std::mem::take(&mut chat.draft));
                                chat.scroll = 0;
                            }
                            KeyCode::Backspace => {
                                chat.draft.pop();
                            }
                            KeyCode::Char(c)
                                if !key
                                    .modifiers
                                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                            {
                                chat.draft.push(c)
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        true
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
        let mut x = inner.x;
        for tab in &self.tabs {
            let width = if *tab == Tab::Info { 6 } else { 8 };
            let rect = Rect::new(x, inner.y, width, 1);
            frame.render_widget(
                Paragraph::new(tab.label()).style(theme.fg(if *tab == self.active {
                    theme.rose
                } else {
                    theme.text_dim
                })),
                rect,
            );
            hits.push((rect, Action::Tab(*tab)));
            if *tab != Tab::Info {
                let close = Rect::new(x + width - 2, inner.y, 1, 1);
                frame.render_widget(Paragraph::new("×"), close);
                hits.insert(0, (close, Action::Close(*tab)));
            }
            x += width;
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
        let body = Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(3),
        );
        hits.push((body, Action::Focus));
        match self.active {
            Tab::Info => {
                let unknown = i18n::tr("right.unknown", "未知");
                let session = model.sessions.get(model.index);
                let p = model.projection;
                let lines = vec![
                    format!(
                        "{}: {}",
                        i18n::tr("right.title", "会话"),
                        session.and_then(|s| s.title.as_deref()).unwrap_or(&unknown)
                    ),
                    format!(
                        "ID: {}",
                        session
                            .map(|s| s.id.to_string())
                            .unwrap_or_else(|| unknown.clone())
                    ),
                    format!(
                        "{}: {}",
                        i18n::tr("right.project", "项目"),
                        model.workspace.display()
                    ),
                    format!(
                        "{}: {}",
                        i18n::tr("right.state", "状态"),
                        if model.running {
                            i18n::tr("right.running", "运行中…")
                        } else {
                            i18n::tr("right.idle", "空闲")
                        }
                    ),
                    format!(
                        "{}: {:?}",
                        i18n::tr("right.kernel", "内核"),
                        p.map(|p| p.harness).unwrap_or_default()
                    ),
                    format!(
                        "{}: {}",
                        i18n::tr("right.provider", "供应商"),
                        if model.provider.is_empty() {
                            unknown.as_str()
                        } else {
                            model.provider
                        }
                    ),
                    format!(
                        "{}: {}",
                        i18n::tr("right.model", "模型"),
                        if model.model.is_empty() {
                            unknown.as_str()
                        } else {
                            model.model
                        }
                    ),
                    format!(
                        "{}: {}",
                        i18n::tr("right.permission", "权限"),
                        i18n::permission_name(model.permission_mode)
                    ),
                    format!(
                        "{}: {} / {}",
                        i18n::tr("right.tokens", "Token 输入 / 输出"),
                        p.map(|p| p.input_tokens).unwrap_or(0),
                        p.map(|p| p.output_tokens).unwrap_or(0)
                    ),
                    format!("{}: {}", i18n::tr("right.context", "上下文"), unknown),
                ];
                let button_y = body.bottom().saturating_sub(4);
                let info = Rect::new(body.x, body.y, body.width, button_y.saturating_sub(body.y));
                frame.render_widget(
                    Paragraph::new(lines.join("\n"))
                        .scroll((self.info_scroll.min(u16::MAX as usize) as u16, 0))
                        .wrap(Wrap { trim: false })
                        .style(theme.fg(theme.text)),
                    info,
                );
                for (i, tab) in [Tab::Files, Tab::Terminal, Tab::Chat].iter().enumerate() {
                    let rect = Rect::new(body.x + 1, button_y + i as u16, body.width - 2, 1);
                    let label = if *tab == Tab::Chat {
                        i18n::tr("right.session_title", "侧边对话")
                    } else {
                        tab.label()
                    };
                    frame.render_widget(
                        Paragraph::new(format!("[ {label} ]")).style(theme.fg(theme.rose)),
                        rect,
                    );
                    hits.insert(0, (rect, Action::Tab(*tab)));
                }
            }
            Tab::Files => {
                let back = Rect::new(body.x, body.y, body.width, 1);
                frame.render_widget(
                    Paragraph::new(format!(
                        "‹ {}",
                        self.files
                            .directory
                            .strip_prefix(&self.files.root)
                            .unwrap_or(&self.files.directory)
                            .display()
                    ))
                    .style(theme.fg(theme.rose)),
                    back,
                );
                hits.insert(0, (back, Action::Back));
                let content = Rect::new(
                    body.x,
                    body.y + 1,
                    body.width,
                    body.height.saturating_sub(1),
                );
                if let Some(preview) = &self.files.preview {
                    frame.render_widget(
                        Paragraph::new(preview.as_str())
                            .scroll((self.files.scroll.min(u16::MAX as usize) as u16, 0)),
                        content,
                    );
                } else if !self.files.message.is_empty() {
                    frame.render_widget(
                        Paragraph::new(self.files.message.as_str()).wrap(Wrap { trim: false }),
                        content,
                    );
                } else {
                    let start = self
                        .files
                        .selected
                        .saturating_sub(content.height.saturating_sub(1) as usize);
                    for (row, (index, entry)) in self
                        .files
                        .entries
                        .iter()
                        .enumerate()
                        .skip(start)
                        .take(content.height as usize)
                        .enumerate()
                    {
                        let rect = Rect::new(content.x, content.y + row as u16, content.width, 1);
                        let label = format!(
                            "{} {}{}",
                            if index == self.files.selected {
                                "›"
                            } else {
                                " "
                            },
                            entry.path.file_name().unwrap_or_default().to_string_lossy(),
                            if entry.directory { "/" } else { "" }
                        );
                        frame.render_widget(
                            Paragraph::new(label).style(theme.fg(
                                if index == self.files.selected {
                                    theme.rose
                                } else {
                                    theme.text
                                },
                            )),
                            rect,
                        );
                        hits.insert(0, (rect, Action::File(index)));
                    }
                }
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
                frame.render_widget(
                    Paragraph::new(terminal_label).style(theme.fg(theme.text_dim)),
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
                    if self.focused && !screen.hide_cursor() {
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
                    let header = format!(
                        "{} · Blora\n{}\n{}",
                        chat.id,
                        chat.workspace.display(),
                        chat.status
                    );
                    frame.render_widget(
                        Paragraph::new(header).style(theme.fg(theme.text_dim)),
                        Rect::new(body.x, body.y, body.width, 3),
                    );
                    let content = Rect::new(
                        body.x,
                        body.y + 3,
                        body.width,
                        body.height.saturating_sub(6),
                    );
                    let lines = chat
                        .projection
                        .transcript
                        .iter()
                        .map(|item| match item {
                            TranscriptItem::User { text, .. } => format!("› {text}"),
                            TranscriptItem::Assistant { text, .. } => text.clone(),
                            TranscriptItem::Tool { name, status, .. } => {
                                format!("[{name}: {status}]")
                            }
                            TranscriptItem::System { summary, .. } => summary.clone(),
                            TranscriptItem::Question { question, .. } => format!("? {question}"),
                            _ => String::new(),
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let line_count = lines
                        .lines()
                        .map(|line| {
                            unicode_width::UnicodeWidthStr::width(line)
                                .max(1)
                                .div_ceil(content.width.max(1) as usize)
                        })
                        .sum::<usize>();
                    let wrapped = Paragraph::new(lines).wrap(Wrap { trim: false });
                    let offset = line_count
                        .saturating_sub(content.height as usize)
                        .saturating_sub(chat.scroll);
                    frame.render_widget(
                        wrapped.scroll((offset.min(u16::MAX as usize) as u16, 0)),
                        content,
                    );
                    let draft = Rect::new(body.x, body.bottom().saturating_sub(3), body.width, 3);
                    frame.render_widget(
                        Paragraph::new(format!("› {}", chat.draft))
                            .wrap(Wrap { trim: false })
                            .style(theme.fg(theme.text)),
                        draft,
                    );
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
}
