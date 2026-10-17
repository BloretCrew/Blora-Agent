// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Frame layout: open canvas, focused composer, dim chrome.

use std::path::Path;

use blora_policy::PermissionMode;
use blora_session::{SessionProjection, TranscriptItem};
use blora_storage::{ApprovalRecord, SessionSummary};
use blora_types::Mode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::dialogs::DialogId;
use crate::i18n;
use crate::selection::Selection;
use crate::slash::{self, SlashCommand};
use crate::theme::{self, Scheme, Theme, ThemePref};

const PAD: u16 = 2;
/// Work indicator: ping-pong through this star sequence.
const SPINNER: &[char] = &[
    '✦', '✧', '✩', '✪', '✫', '✬', '✭', '✮', '✯', '✰', '✴', '✵', '✶', '✷', '✸', '✹', '✺', '✻', '✼',
    '✽',
];

/// Bloret PassPort device-flow login dialog shown while a login is pending.
#[derive(Clone, Debug)]
pub struct PassportDialog {
    pub user_code: String,
    pub verification_uri: String,
    /// True once the browser hand-off has been attempted; switches the footer.
    pub opened_browser: bool,
}

/// One selectable provider inside the provider-switch dialog.
#[derive(Clone, Debug)]
pub struct ProviderOption {
    /// Value stored in the provider override (`""` = follow the default).
    pub id: String,
    /// Display name, e.g. `Blora` or `OpenAI`.
    pub display: String,
    /// One-line description, e.g. the required credential or current state.
    pub hint: String,
    /// False when the provider cannot be used right now (no key, not logged in).
    pub available: bool,
    pub models: Vec<ModelOption>,
}

/// One selectable model in the right pane of the provider dialog.
#[derive(Clone, Debug)]
pub struct ModelOption {
    pub id: String,
    pub display: String,
    pub hint: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderPane {
    Providers,
    Models,
}

/// Model-provider switch dialog shown via `/provider` with no arguments.
#[derive(Clone, Debug)]
pub struct ProviderDialog {
    pub options: Vec<ProviderOption>,
    pub selected: usize,
    pub pane: ProviderPane,
    pub model_selected: usize,
    pub fullscreen: bool,
    pub minimized: bool,
}

impl ProviderDialog {
    #[must_use]
    pub fn current(&self) -> Option<&ProviderOption> {
        self.options.get(self.selected)
    }

    #[must_use]
    pub fn is_add(&self) -> bool {
        self.current()
            .is_some_and(|option| option.id == blora_catalog::ADD_PROVIDER_ID)
    }

    #[must_use]
    pub fn current_models(&self) -> &[ModelOption] {
        self.current()
            .map(|option| option.models.as_slice())
            .unwrap_or(&[])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddProviderStep {
    Catalog,
    CustomId,
    CustomBase,
    ApiKey,
    Review,
}

/// Wizard for adding a provider from models.dev (or a custom endpoint).
#[derive(Clone, Debug)]
pub struct AddProviderDialog {
    pub step: AddProviderStep,
    pub catalog: Vec<blora_catalog::CatalogEntry>,
    pub selected: usize,
    pub filter: String,
    pub custom_id: String,
    pub custom_base: String,
    pub api_key: String,
    pub format: blora_catalog::MessageFormat,
    pub preview_models: Vec<blora_catalog::CatalogModel>,
    pub resolved_base: String,
    pub error: Option<String>,
    pub fullscreen: bool,
    pub minimized: bool,
}

impl AddProviderDialog {
    #[must_use]
    pub fn is_custom(&self) -> bool {
        self.selected == 0
    }

    #[must_use]
    pub fn filtered_catalog(&self) -> Vec<&blora_catalog::CatalogEntry> {
        let query = self.filter.trim().to_ascii_lowercase();
        self.catalog
            .iter()
            .filter(|entry| {
                if query.is_empty() {
                    return true;
                }
                entry.id.to_ascii_lowercase().contains(&query)
                    || entry.name.to_ascii_lowercase().contains(&query)
                    || entry
                        .api
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains(&query)
            })
            .collect()
    }

    #[must_use]
    pub fn catalog_entry(&self) -> Option<&blora_catalog::CatalogEntry> {
        if self.is_custom() {
            None
        } else {
            self.filtered_catalog()
                .into_iter()
                .nth(self.selected.saturating_sub(1))
        }
    }

    #[must_use]
    pub fn catalog_len(&self) -> usize {
        self.filtered_catalog().len().saturating_add(1)
    }

    pub fn clamp_catalog_selected(&mut self) {
        let max = self.catalog_len().saturating_sub(1);
        if self.selected > max {
            self.selected = max;
        }
    }

    /// Custom vendors, and catalog entries with no `api` field, need a base URL.
    #[must_use]
    pub fn needs_base_step(&self) -> bool {
        self.is_custom()
            || self
                .catalog_entry()
                .is_some_and(|entry| entry.api.as_ref().is_none_or(|api| api.trim().is_empty()))
    }
}

/// One selectable scheme inside the theme picker.
#[derive(Clone, Debug)]
pub struct ThemeOption {
    /// Palette id (`coral`, `indigo`, …) or scheme id (`dark`, `light`, …).
    pub id: String,
    pub display: String,
    pub hint: String,
    /// Five-swatch preview from Blora Design `THEME_PRESETS`.
    pub swatches: Vec<Color>,
}

/// Color-scheme picker shown via `/theme` with no arguments.
#[derive(Clone, Debug)]
pub struct ThemeDialog {
    pub options: Vec<ThemeOption>,
    pub selected: usize,
    /// Auto / light / dark section currently shown.
    pub scheme: Scheme,
    /// Preference restored if the picker is cancelled.
    pub original: ThemePref,
    pub fullscreen: bool,
    pub minimized: bool,
}

#[derive(Clone, Debug)]
pub struct ModeMenu {
    pub selected: usize,
    pub fullscreen: bool,
    pub minimized: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ModeOption {
    pub mode: Mode,
    pub title: &'static str,
    pub description: &'static str,
}

pub const MODE_OPTIONS: [ModeOption; 4] = [
    ModeOption {
        mode: Mode::Code,
        title: "Code · 编码",
        description: "读改代码、运行测试与 Git，像结对工程师一样完成开发任务。",
    },
    ModeOption {
        mode: Mode::Work,
        title: "Work · 工作产出",
        description: "联网调研、读取 PDF/Office 文档，产出报告、表格等成果文件。",
    },
    ModeOption {
        mode: Mode::Agent,
        title: "Agent · 自主助理",
        description: "端到端推进目标，委派与定时执行，自动沉淀技能、记忆并检索历史。",
    },
    ModeOption {
        mode: Mode::Imagine,
        title: "Imagine",
        description: "根据描述生成图片，并保存在工作区。",
    },
];

impl<'a> FrameModel<'a> {
    fn theme_name(&self) -> &'static str {
        theme::palette().name_zh()
    }
}

impl ModeMenu {
    #[must_use]
    pub fn current(&self) -> ModeOption {
        MODE_OPTIONS[self.selected.min(MODE_OPTIONS.len() - 1)]
    }
}

#[derive(Clone, Debug)]
pub struct ProjectPicker {
    pub selected: usize,
    pub fullscreen: bool,
    pub minimized: bool,
}

#[derive(Clone, Debug)]
pub struct FolderPicker {
    pub path: std::path::PathBuf,
    pub entries: Vec<std::path::PathBuf>,
    pub selected: usize,
    pub scroll: usize,
    pub fullscreen: bool,
    pub minimized: bool,
}

pub use crate::session_picker::SessionPicker;

#[derive(Clone, Debug)]
pub struct ToolCallDetail {
    pub name: String,
    pub status: String,
    pub arguments: Option<String>,
    pub output: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ToolDialog {
    pub tools: Vec<ToolCallDetail>,
    pub scroll: usize,
    pub fullscreen: bool,
    pub minimized: bool,
}

#[derive(Clone, Debug)]
pub struct ToolDetailDialog {
    pub tool: ToolCallDetail,
    pub scroll: usize,
    pub fullscreen: bool,
    pub minimized: bool,
}

#[derive(Clone, Debug)]
pub struct GitStatusInfo {
    pub branch: String,
    pub added: usize,
    pub removed: usize,
    pub ahead: usize,
    pub behind: usize,
    pub stashes: usize,
    pub clean: bool,
    pub raw: String,
    pub diff: String,
    pub log: String,
}

#[derive(Clone, Debug)]
pub struct GitDialog {
    pub info: GitStatusInfo,
    pub page: usize,
    pub selected: usize,
    pub message: String,
    pub editing_message: bool,
    pub generating_message: bool,
    pub syncing: bool,
    pub excluded_files: std::collections::HashSet<String>,
    pub diff_path: Option<String>,
    pub diff_text: String,
    pub diff_scroll: usize,
    pub scroll: usize,
    pub feedback: Option<String>,
    pub fullscreen: bool,
    pub minimized: bool,
}

#[derive(Clone, Debug)]
pub struct ContextDialog {
    pub scroll: usize,
    /// `0` current context, `1` overview, `2` models.
    pub page: usize,
    /// `0` all time, `1` last 7 days, `2` last 30 days.
    pub range: usize,
    pub fullscreen: bool,
    pub minimized: bool,
    pub activity: blora_storage::UsageActivity,
}

impl ContextDialog {
    pub fn set_page(&mut self, page: usize) {
        let page = page % 3;
        if page != self.page {
            self.page = page;
            self.scroll = 0;
        }
    }

    pub fn cycle_page(&mut self, step: isize) {
        let next = (self.page as isize + step).rem_euclid(3);
        self.set_page(usize::try_from(next).unwrap_or(0));
    }

    pub fn set_range(&mut self, range: usize) {
        let range = range % 3;
        if range != self.range {
            self.range = range;
            self.scroll = 0;
        }
    }

    /// Cycles the date filter on the overview and model pages.
    pub fn cycle_range(&mut self) {
        if self.page == 0 {
            return;
        }
        self.set_range(self.range + 1);
    }
}

#[derive(Clone, Debug)]
pub struct SettingsDialog {
    pub page: usize,
    pub selected: usize,
    pub fullscreen: bool,
    pub minimized: bool,
}

#[derive(Clone, Debug)]
pub struct HarnessDialog {
    pub selected: usize,
    pub available: Vec<bool>,
    pub fullscreen: bool,
    pub minimized: bool,
}

impl HarnessDialog {
    pub fn new(current: blora_types::HarnessKind) -> Self {
        let kinds = blora_types::HarnessKind::all();
        Self {
            selected: kinds.iter().position(|kind| *kind == current).unwrap_or(0),
            available: kinds
                .into_iter()
                .map(|kind| blora_harness::detect(kind).is_some())
                .collect(),
            fullscreen: false,
            minimized: false,
        }
    }

    pub fn select(&mut self, index: usize) -> Option<blora_types::HarnessKind> {
        if self.minimized || !self.available.get(index).copied().unwrap_or(false) {
            return None;
        }
        self.selected = index;
        blora_types::HarnessKind::all().get(index).copied()
    }

    pub fn key(&mut self, key: crossterm::event::KeyCode) -> Option<blora_types::HarnessKind> {
        use crossterm::event::KeyCode;
        if self.minimized {
            if key == KeyCode::Enter {
                self.minimized = false;
            }
            return None;
        }
        match key {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                self.selected = (self.selected + 1)
                    .min(blora_types::HarnessKind::all().len().saturating_sub(1));
            }
            KeyCode::Enter => return self.select(self.selected),
            _ => {}
        }
        None
    }
}

#[derive(Clone, Debug)]
pub struct PermissionDialog {
    pub selected: usize,
    pub confirm_bypass: bool,
    pub fullscreen: bool,
    pub minimized: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PermissionAction {
    None,
    Close,
    Apply(PermissionMode),
}

impl PermissionDialog {
    pub fn bypass_warning() -> Self {
        let mut dialog = Self::new(PermissionMode::Yolo);
        dialog.confirm_bypass = true;
        dialog
    }

    pub fn select(&mut self, index: usize) -> PermissionAction {
        if self.confirm_bypass || index >= PermissionMode::ALL.len() {
            return PermissionAction::None;
        }
        self.selected = index;
        let mode = PermissionMode::ALL[index];
        if mode == PermissionMode::Yolo {
            self.confirm_bypass = true;
            PermissionAction::None
        } else {
            PermissionAction::Apply(mode)
        }
    }

    pub fn new(current: PermissionMode) -> Self {
        Self {
            confirm_bypass: false,
            selected: PermissionMode::ALL
                .iter()
                .position(|mode| *mode == current)
                .unwrap_or(0),
            fullscreen: false,
            minimized: false,
        }
    }

    pub fn key(&mut self, key: crossterm::event::KeyCode) -> PermissionAction {
        use crossterm::event::KeyCode;
        if key == KeyCode::Esc {
            return PermissionAction::Close;
        }
        if self.minimized {
            if key == KeyCode::Enter {
                self.minimized = false;
            }
            return PermissionAction::None;
        }
        if self.confirm_bypass {
            return match key {
                KeyCode::Char('y') => PermissionAction::Apply(PermissionMode::Yolo),
                KeyCode::Char('n') => PermissionAction::Close,
                _ => PermissionAction::None,
            };
        }
        match key {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.selected = (self.selected + 1).min(PermissionMode::ALL.len() - 1),
            KeyCode::Enter => return self.select(self.selected),
            _ => {}
        }
        PermissionAction::None
    }
}

fn permission_description(mode: PermissionMode) -> String {
    let (key, fallback) = match mode {
        PermissionMode::Plan => (
            "permission.plan.description",
            "只读探索；仅可写 .blora/plan.md，禁止修改命令",
        ),
        PermissionMode::Ask => (
            "permission.default.description",
            "文件写入与修改命令均需确认",
        ),
        PermissionMode::AutoEdit => (
            "permission.accept_edits.description",
            "工作区文件编辑免确认；修改命令仍需确认",
        ),
        PermissionMode::Yolo => (
            "permission.bypass.description",
            "跳过所有审批（含危险命令）；硬性网络与沙盒限制保留",
        ),
    };
    i18n::tr(key, fallback)
}

#[derive(Clone)]
pub struct FrameModel<'a> {
    pub dialog_stack: &'a [DialogId],
    pub workspace: &'a Path,
    pub sessions: &'a [SessionSummary],
    pub index: usize,
    pub sidebar: &'a SidebarState,
    pub right_panel: Option<&'a crate::right_panel::RightPanel>,
    pub projection: Option<&'a SessionProjection>,
    pub pending: &'a [ApprovalRecord],
    pub input: &'a str,
    pub status: &'a str,
    pub notice: Option<&'a str>,
    pub paste_preview: Option<&'a str>,
    /// Pending PassPort login; renders as a centered modal dialog.
    pub passport_dialog: Option<&'a PassportDialog>,
    /// Model-provider switch dialog; renders as a centered modal dialog.
    pub provider_dialog: Option<&'a ProviderDialog>,
    /// Add-provider wizard; takes render priority over the switch dialog.
    pub add_provider_dialog: Option<&'a AddProviderDialog>,
    /// Color-scheme picker; renders as a centered modal dialog.
    pub theme_dialog: Option<&'a ThemeDialog>,
    pub context_dialog: Option<&'a ContextDialog>,
    pub harness_dialog: Option<&'a HarnessDialog>,
    pub permission_dialog: Option<&'a PermissionDialog>,
    pub settings_dialog: Option<&'a SettingsDialog>,
    pub tool_dialog: Option<&'a ToolDialog>,
    pub tool_detail_dialog: Option<&'a ToolDetailDialog>,
    pub slash_hits: &'a [&'static SlashCommand],
    pub slash_selected: usize,
    pub(crate) mentions: Option<&'a crate::session_mentions::SessionMentions>,
    pub search: Option<&'a str>,
    pub hide_tools: bool,
    pub message_roles: bool,
    pub scroll: usize,
    pub permission_mode: PermissionMode,
    pub model: &'a str,
    pub provider: &'a str,
    pub mode_menu: Option<&'a ModeMenu>,
    pub project_picker: Option<&'a ProjectPicker>,
    pub folder_picker: Option<&'a FolderPicker>,
    pub question_dialog: Option<&'a crate::question::QuestionForm>,
    pub session_picker: Option<&'a SessionPicker>,
    pub git_dialog: Option<&'a GitDialog>,
    pub git_status: Option<&'a GitStatusInfo>,
    pub user_label: &'a str,
    pub running: bool,
    pub tick: u64,
    pub pointer: Option<(u16, u16)>,
    pub text_selection: Option<Selection>,
}

/// A clickable region from the last painted frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Transcript,
    SidebarSession(usize),
    SidebarSettings,
    ToolSummary(Vec<usize>),
    SideToolSummary(blora_types::SessionId, Vec<usize>),
    ToolDetailRow(usize),
    GitStatus,
    GitTab(usize),
    GitMessage,
    GitGenerate,
    GitPrimary,
    GitFile(usize),
    GitRow(usize),
    GitAction(usize),
    Composer,
    Slash(usize),
    Allow,
    Deny,
    PrevSession,
    NextSession,
    ProjectPicker,
    ProjectPickerRow(usize),
    FolderPickerRow(usize),
    FolderPickerOpen,
    SessionPicker,
    SessionPickerRow(usize),
    SessionSearch,
    Mode,
    ModeRow(usize),
    CancelRun,
    ToggleApprove,
    PermissionRow(usize),
    Harness,
    HarnessRow(usize),
    ContextUsage,
    ContextTab(usize),
    ContextRange(usize),
    ProviderTarget,
    Hint(HintAction),
    Notice,
    /// macOS-style window dots on the login dialog.
    TrafficClose,
    TrafficMinimize,
    TrafficOpenBrowser,
    /// A provider row inside the provider-switch dialog.
    ProviderRow(usize),
    /// A model row in the right pane of the provider-switch dialog.
    ProviderModelRow(usize),
    /// A row in the add-provider catalog list.
    AddProviderRow(usize),
    /// Cycle the message-format step of the add-provider wizard.
    AddProviderFormat,
    /// A scheme row inside the theme picker.
    ThemeRow(usize),
    /// Auto / light / dark section tab (`0` auto, `1` light, `2` dark).
    ThemeTab(usize),
    SettingsTab(usize),
    SettingsRow(usize),
    /// Question title in the horizontal tab row.
    QuestionTab(usize),
    /// Option row inside the question dialog: question index, option index.
    QuestionOption(usize, usize),
    /// Picker row inside the focused file or folder question.
    QuestionRow(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintAction {
    Send,
    Commands,
    SessionNext,
    New,
    Quit,
    Allow,
    Deny,
    Close,
    Complete,
    Run,
}

#[derive(Clone, Debug, Default)]
pub struct HitMap {
    pub sidebar: Option<Rect>,
    pub sidebar_projects: Option<Rect>,
    pub sidebar_sessions: Option<Rect>,
    pub sidebar_project_max: usize,
    pub sidebar_session_max: usize,
    pub sidebar_rows: Vec<(Rect, Hit)>,
    pub modal: bool,
    pub modal_area: Option<Rect>,
    pub transcript: Rect,
    pub side_tool_summary_rows: Vec<(Rect, blora_types::SessionId, Vec<usize>)>,
    pub tool_summary_rows: Vec<(Rect, Vec<usize>)>,
    pub composer: Rect,
    pub overlay: Option<Rect>,
    pub notice: bool,
    pub slash_open: bool,
    pub slash_rows: Vec<(Rect, usize)>,
    pub allow: Option<Rect>,
    pub deny: Option<Rect>,
    pub prev_session: Option<Rect>,
    pub next_session: Option<Rect>,
    pub project_picker: Option<Rect>,
    pub project_picker_rows: Vec<(Rect, usize)>,
    pub folder_picker_rows: Vec<(Rect, usize)>,
    pub folder_picker_open: Option<Rect>,
    pub question_tabs: Vec<(Rect, usize)>,
    pub question_options: Vec<(Rect, usize, usize)>,
    pub question_rows: Vec<(Rect, usize)>,
    pub session_picker: Option<Rect>,
    pub session_search: Option<Rect>,
    pub session_picker_rows: Vec<(Rect, usize)>,
    pub mode: Option<Rect>,
    pub mode_rows: Vec<(Rect, usize)>,
    pub cancel_run: Option<Rect>,
    pub toggle_approve: Option<Rect>,
    pub provider_target: Option<Rect>,
    pub context_usage: Option<Rect>,
    pub context_tabs: Vec<(Rect, usize)>,
    pub context_ranges: Vec<(Rect, usize)>,
    pub git_status: Option<Rect>,
    pub git_tabs: Vec<(Rect, usize)>,
    pub git_message: Option<Rect>,
    pub git_generate: Option<Rect>,
    pub git_primary: Option<Rect>,
    pub git_files: Vec<(Rect, usize)>,
    pub git_diff: Option<Rect>,
    pub git_rows: Vec<(Rect, usize)>,
    pub git_actions: Vec<(Rect, usize)>,
    pub hints: Vec<(Rect, HintAction)>,
    /// macOS-style traffic-light dots of the login dialog.
    pub traffic_lights: [Option<Rect>; 3],
    /// Provider rows of the provider-switch dialog.
    pub provider_rows: Vec<(Rect, usize)>,
    /// Model rows of the provider-switch dialog.
    pub provider_model_rows: Vec<(Rect, usize)>,
    pub tool_detail_rows: Vec<(Rect, usize)>,
    /// Catalog rows of the add-provider wizard.
    pub add_provider_rows: Vec<(Rect, usize)>,
    /// Scheme rows of the theme picker.
    pub theme_rows: Vec<(Rect, usize)>,
    /// Auto / light / dark section tabs.
    pub theme_tabs: Vec<(Rect, usize)>,
    pub harness: Option<Rect>,
    pub harness_rows: Vec<(Rect, usize)>,
    pub permission_rows: Vec<(Rect, usize)>,
    pub settings_rows: Vec<(Rect, usize)>,
    pub settings_tabs: Vec<(Rect, usize)>,
}

impl HitMap {
    #[must_use]
    pub fn hit(&self, col: u16, row: u16) -> Option<Hit> {
        if self.modal && !self.modal_area.is_some_and(|rect| contains(rect, col, row)) {
            return None;
        }
        for (rect, hit) in &self.sidebar_rows {
            if contains(*rect, col, row) {
                return Some(hit.clone());
            }
        }
        for (index, rect) in self.traffic_lights.iter().enumerate() {
            if let Some(rect) = rect
                && contains(*rect, col, row)
            {
                return Some(match index {
                    0 => Hit::TrafficClose,
                    1 => Hit::TrafficMinimize,
                    _ => Hit::TrafficOpenBrowser,
                });
            }
        }
        for (rect, idx) in &self.git_tabs {
            if contains(*rect, col, row) {
                return Some(Hit::GitTab(*idx));
            }
        }
        if self
            .git_message
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::GitMessage);
        }
        if self
            .git_generate
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::GitGenerate);
        }
        if self
            .git_primary
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::GitPrimary);
        }
        for (rect, idx) in &self.git_files {
            if contains(*rect, col, row) {
                return Some(Hit::GitFile(*idx));
            }
        }
        for (rect, idx) in &self.git_rows {
            if contains(*rect, col, row) {
                return Some(Hit::GitRow(*idx));
            }
        }
        for (rect, idx) in &self.git_actions {
            if contains(*rect, col, row) {
                return Some(Hit::GitAction(*idx));
            }
        }
        if self.git_status.is_some_and(|rect| contains(rect, col, row)) {
            return Some(Hit::GitStatus);
        }
        for (rect, idx) in &self.tool_detail_rows {
            if contains(*rect, col, row) {
                return Some(Hit::ToolDetailRow(*idx));
            }
        }
        for (rect, owner, indices) in &self.side_tool_summary_rows {
            if contains(*rect, col, row) {
                return Some(Hit::SideToolSummary(owner.clone(), indices.clone()));
            }
        }
        for (rect, indices) in &self.tool_summary_rows {
            if contains(*rect, col, row) {
                return Some(Hit::ToolSummary(indices.clone()));
            }
        }
        for (rect, idx) in &self.slash_rows {
            if contains(*rect, col, row) {
                return Some(Hit::Slash(*idx));
            }
        }
        for (rect, idx) in &self.provider_model_rows {
            if contains(*rect, col, row) {
                return Some(Hit::ProviderModelRow(*idx));
            }
        }
        for (rect, idx) in &self.provider_rows {
            if contains(*rect, col, row) {
                return Some(Hit::ProviderRow(*idx));
            }
        }
        for (rect, idx) in &self.add_provider_rows {
            if contains(*rect, col, row) {
                return Some(Hit::AddProviderRow(*idx));
            }
        }
        for (rect, idx) in &self.theme_tabs {
            if contains(*rect, col, row) {
                return Some(Hit::ThemeTab(*idx));
            }
        }
        for (rect, idx) in &self.settings_tabs {
            if contains(*rect, col, row) {
                return Some(Hit::SettingsTab(*idx));
            }
        }
        for (rect, idx) in &self.context_tabs {
            if contains(*rect, col, row) {
                return Some(Hit::ContextTab(*idx));
            }
        }
        for (rect, idx) in &self.context_ranges {
            if contains(*rect, col, row) {
                return Some(Hit::ContextRange(*idx));
            }
        }
        if self.harness.is_some_and(|rect| contains(rect, col, row)) {
            return Some(Hit::Harness);
        }
        for (rect, idx) in &self.harness_rows {
            if contains(*rect, col, row) {
                return Some(Hit::HarnessRow(*idx));
            }
        }
        for (rect, idx) in &self.permission_rows {
            if contains(*rect, col, row) {
                return Some(Hit::PermissionRow(*idx));
            }
        }
        for (rect, idx) in &self.settings_rows {
            if contains(*rect, col, row) {
                return Some(Hit::SettingsRow(*idx));
            }
        }
        for (rect, idx) in &self.theme_rows {
            if contains(*rect, col, row) {
                return Some(Hit::ThemeRow(*idx));
            }
        }
        for (rect, idx) in &self.mode_rows {
            if contains(*rect, col, row) {
                return Some(Hit::ModeRow(*idx));
            }
        }
        for (rect, idx) in &self.project_picker_rows {
            if contains(*rect, col, row) {
                return Some(Hit::ProjectPickerRow(*idx));
            }
        }
        for (rect, idx) in &self.question_tabs {
            if contains(*rect, col, row) {
                return Some(Hit::QuestionTab(*idx));
            }
        }
        for (rect, question, option) in &self.question_options {
            if contains(*rect, col, row) {
                return Some(Hit::QuestionOption(*question, *option));
            }
        }
        for (rect, idx) in &self.question_rows {
            if contains(*rect, col, row) {
                return Some(Hit::QuestionRow(*idx));
            }
        }
        for (rect, idx) in &self.folder_picker_rows {
            if contains(*rect, col, row) {
                return Some(Hit::FolderPickerRow(*idx));
            }
        }
        if self
            .folder_picker_open
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::FolderPickerOpen);
        }
        if self
            .project_picker
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::ProjectPicker);
        }
        if self
            .session_search
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::SessionSearch);
        }
        for (rect, idx) in &self.session_picker_rows {
            if contains(*rect, col, row) {
                return Some(Hit::SessionPickerRow(*idx));
            }
        }
        if self
            .session_picker
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::SessionPicker);
        }
        if self.mode.is_some_and(|rect| contains(rect, col, row)) {
            return Some(Hit::Mode);
        }
        if self.notice && self.overlay.is_some_and(|rect| contains(rect, col, row)) {
            return Some(Hit::Notice);
        }
        if self.allow.is_some_and(|rect| contains(rect, col, row)) {
            return Some(Hit::Allow);
        }
        if self.deny.is_some_and(|rect| contains(rect, col, row)) {
            return Some(Hit::Deny);
        }
        for (rect, action) in &self.hints {
            if contains(*rect, col, row) {
                return Some(Hit::Hint(*action));
            }
        }
        if self
            .toggle_approve
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::ToggleApprove);
        }
        if self
            .provider_target
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::ProviderTarget);
        }
        if self
            .context_usage
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::ContextUsage);
        }
        if self.cancel_run.is_some_and(|rect| contains(rect, col, row)) {
            return Some(Hit::CancelRun);
        }
        if self
            .prev_session
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::PrevSession);
        }
        if self
            .next_session
            .is_some_and(|rect| contains(rect, col, row))
        {
            return Some(Hit::NextSession);
        }
        if contains(self.composer, col, row) {
            return Some(Hit::Composer);
        }
        if contains(self.transcript, col, row) {
            return Some(Hit::Transcript);
        }
        None
    }

    #[must_use]
    pub fn over_slash(&self, col: u16, row: u16) -> bool {
        self.overlay
            .is_some_and(|rect| self.slash_open && contains(rect, col, row))
    }
}

fn contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

#[derive(Clone, Debug, Default)]
pub struct SidebarState {
    pub projects: usize,
    pub sessions: usize,
    active: Option<String>,
    workspace: Option<String>,
    height: u16,
}

pub(crate) fn canvas_regions(area: Rect) -> (Option<Rect>, Rect) {
    if area.width < 100 || area.height < 18 {
        return (None, area);
    }
    (
        Some(Rect::new(area.x, area.y, 28, area.height)),
        Rect::new(area.x + 28, area.y, area.width - 28, area.height),
    )
}

fn sidebar_list_heights(height: u16) -> (usize, usize) {
    let available = height.saturating_sub(11) as usize;
    let projects = (available / 3).clamp(1, 4);
    (projects, available.saturating_sub(projects))
}

fn sidebar_session_indices(sessions: &[SessionSummary], workspace: &Path) -> Vec<usize> {
    sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| Path::new(&session.workspace_path) == workspace)
        .map(|(index, _)| index)
        .collect()
}

impl SidebarState {
    pub fn sync(
        &mut self,
        sessions: &[SessionSummary],
        index: usize,
        workspace: &Path,
        height: u16,
    ) {
        let active_workspace = sessions
            .get(index)
            .map(|session| Path::new(&session.workspace_path))
            .unwrap_or(workspace);
        let active = sessions.get(index).map(|session| session.id.to_string());
        let path = active_workspace.display().to_string();
        let (project_height, session_height) = sidebar_list_heights(height);
        let projects = crate::project_paths(sessions);
        let indices = sidebar_session_indices(sessions, active_workspace);
        if self.workspace.as_ref() != Some(&path) {
            self.sessions = 0;
            if let Some(position) = projects.iter().position(|project| project == &path) {
                reveal_sidebar_row(&mut self.projects, position, project_height);
            }
        }
        if self.active != active || self.workspace.as_ref() != Some(&path) || self.height != height
        {
            if let Some(position) = indices.iter().position(|target| *target == index) {
                reveal_sidebar_row(&mut self.sessions, position, session_height);
            }
        }
        self.projects = self
            .projects
            .min(projects.len().saturating_sub(project_height));
        self.sessions = self
            .sessions
            .min(indices.len().saturating_sub(session_height));
        self.height = height;
        self.active = active;
        self.workspace = Some(path);
    }

    pub fn scroll(&mut self, hits: &HitMap, col: u16, row: u16, up: bool) -> bool {
        if hits.modal || !hits.sidebar.is_some_and(|rect| contains(rect, col, row)) {
            return false;
        }
        let target = if hits
            .sidebar_projects
            .is_some_and(|rect| contains(rect, col, row))
        {
            Some((&mut self.projects, hits.sidebar_project_max))
        } else if hits
            .sidebar_sessions
            .is_some_and(|rect| contains(rect, col, row))
        {
            Some((&mut self.sessions, hits.sidebar_session_max))
        } else {
            None
        };
        if let Some((offset, max)) = target {
            *offset = if up {
                offset.saturating_sub(3)
            } else {
                offset.saturating_add(3).min(max)
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

fn render_sidebar(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    hits.sidebar = Some(area);
    frame.render_widget(
        Block::default().style(theme.base().bg(theme.bg_raised)),
        area,
    );
    for y in area.y..area.bottom() {
        frame.render_widget(
            Paragraph::new("│").style(theme.fg(theme.hairline)),
            Rect::new(area.right() - 1, y, 1, 1),
        );
    }
    let row = |y| Rect::new(area.x + 1, area.y + y, area.width - 3, 1);
    frame.render_widget(
        Paragraph::new("  ✦ Blora").style(theme.rose_bold().bg(theme.bg_raised)),
        row(1),
    );
    sidebar_row(
        frame,
        row(3),
        &i18n::tr("sidebar.new", "新建会话"),
        Some(Hit::Hint(HintAction::New)),
        false,
        model,
        theme,
        hits,
    );
    sidebar_row(
        frame,
        row(4),
        &i18n::tr("sidebar.search", "搜索会话"),
        Some(Hit::SessionPicker),
        false,
        model,
        theme,
        hits,
    );
    sidebar_row(
        frame,
        row(6),
        &format!("{} ›", i18n::tr("sidebar.projects", "项目")),
        Some(Hit::ProjectPicker),
        false,
        model,
        theme,
        hits,
    );
    let active_workspace = model
        .sessions
        .get(model.index)
        .map(|session| Path::new(&session.workspace_path))
        .unwrap_or(model.workspace);
    let projects = crate::project_paths(model.sessions);
    let indices = sidebar_session_indices(model.sessions, active_workspace);
    let (project_height, session_height) = sidebar_list_heights(area.height);
    hits.sidebar_projects = Some(Rect::new(
        area.x,
        area.y + 7,
        area.width - 1,
        project_height as u16,
    ));
    hits.sidebar_sessions = Some(Rect::new(
        area.x,
        area.y + 9 + project_height as u16,
        area.width - 1,
        session_height as u16,
    ));
    hits.sidebar_project_max = projects.len().saturating_sub(project_height);
    hits.sidebar_session_max = indices.len().saturating_sub(session_height);
    for (visible, (index, path)) in projects
        .iter()
        .enumerate()
        .skip(model.sidebar.projects.min(hits.sidebar_project_max))
        .take(project_height)
        .enumerate()
    {
        let name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| path.clone());
        sidebar_row(
            frame,
            row(7 + visible as u16),
            &name,
            Some(Hit::ProjectPickerRow(index)),
            Path::new(path) == active_workspace,
            model,
            theme,
            hits,
        );
    }
    sidebar_row(
        frame,
        row(8 + project_height as u16),
        &i18n::tr("sidebar.recent", "最近会话"),
        None,
        false,
        model,
        theme,
        hits,
    );
    if indices.is_empty() {
        sidebar_row(
            frame,
            row(9 + project_height as u16),
            &i18n::tr("sidebar.empty", "暂无会话"),
            None,
            false,
            model,
            theme,
            hits,
        );
    }
    for (visible, index) in indices
        .iter()
        .skip(model.sidebar.sessions.min(hits.sidebar_session_max))
        .take(session_height)
        .enumerate()
    {
        let title = model
            .projection
            .filter(|_| *index == model.index)
            .and_then(|projection| projection.session.as_ref())
            .and_then(|session| session.title.as_deref())
            .or(model.sessions[*index].title.as_deref())
            .map(str::trim)
            .filter(|title| !title.is_empty() && *title != "tui")
            .map(str::to_owned)
            .unwrap_or_else(|| i18n::tr("session.untitled", "未命名会话"));
        sidebar_row(
            frame,
            row(9 + project_height as u16 + visible as u16),
            &title,
            Some(Hit::SidebarSession(*index)),
            *index == model.index,
            model,
            theme,
            hits,
        );
    }
    sidebar_row(
        frame,
        row(area.height - 2),
        &i18n::tr("sidebar.settings", "设置"),
        Some(Hit::SidebarSettings),
        false,
        model,
        theme,
        hits,
    );
    let user = if model.user_label.trim().is_empty() || model.user_label == "you" {
        i18n::tr("sidebar.guest", "未登录")
    } else {
        model.user_label.to_owned()
    };
    sidebar_row(
        frame,
        row(area.height - 1),
        &user,
        None,
        false,
        model,
        theme,
        hits,
    );
}

pub fn draw(
    frame: &mut Frame<'_>,
    model: &FrameModel<'_>,
    transcript_cache: &mut TranscriptCache,
) -> HitMap {
    let theme = Theme::current();
    let terminal_area = frame.area();
    let (main_area, _) = crate::right_panel::regions(
        terminal_area,
        model.right_panel.is_none_or(|panel| panel.collapsed()),
    );
    let (sidebar, mut area) = canvas_regions(terminal_area);
    area.width = main_area.right().saturating_sub(area.x);
    frame.render_widget(Block::default().style(theme.base()), terminal_area);

    let show_slash = slash::is_open(model.input);
    let mention_height = model.mentions.map_or(0, |mentions| mentions.height());
    let show_notice = !show_slash && mention_height == 0 && model.notice.is_some();
    let show_paste = !show_slash && mention_height == 0 && model.paste_preview.is_some();
    let extra = if show_slash {
        u16::try_from(model.slash_hits.len().clamp(1, slash::MAX_VISIBLE)).unwrap_or(1)
    } else if mention_height > 0 {
        mention_height
    } else if show_notice {
        u16::try_from(model.notice.map(line_count).unwrap_or(1).clamp(1, 12)).unwrap_or(1)
    } else if show_paste {
        u16::try_from(model.paste_preview.map(line_count).unwrap_or(1).clamp(1, 6)).unwrap_or(1)
    } else {
        0
    };
    let show_approval = !model.pending.is_empty();
    let compact = area.height < 14;
    let chrome_height = if compact { 5 } else { 6 };
    let extra = extra.min(
        area.height
            .saturating_sub(chrome_height + u16::from(show_approval) + 3),
    );

    let mut constraints = vec![
        Constraint::Length(1),
        Constraint::Length(u16::from(!compact)),
        Constraint::Min(1),
    ];
    if show_approval {
        constraints.push(Constraint::Length(1));
    }
    if extra > 0 {
        constraints.push(Constraint::Length(extra));
    }
    constraints.push(Constraint::Length(2));
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Length(1));

    let chunks = Layout::vertical(constraints).split(area);
    let mut i = 0;
    let header = chunks[i];
    i += 1;
    let rule = chunks[i];
    i += 1;
    let transcript = chunks[i];
    i += 1;
    let approval = if show_approval {
        let rect = chunks[i];
        i += 1;
        Some(rect)
    } else {
        None
    };
    let overlay = if extra > 0 {
        let rect = chunks[i];
        i += 1;
        Some(rect)
    } else {
        None
    };
    let composer = chunks[i];
    i += 1;
    let status = chunks[i];
    i += 1;
    let hints = chunks[i];

    let mut hits = HitMap {
        transcript: inset(transcript),
        composer: inset(composer),
        overlay: overlay.map(inset),
        notice: show_notice,
        slash_open: show_slash,
        ..HitMap::default()
    };
    if let Some(sidebar) = sidebar {
        render_sidebar(frame, sidebar, model, &theme, &mut hits);
    }
    render_header(frame, header, model, &theme, &mut hits);
    render_rule(frame, rule, &theme);
    render_transcript(
        frame,
        transcript,
        model,
        &theme,
        &mut hits,
        transcript_cache,
    );
    if let Some(rect) = approval {
        render_approval(frame, rect, model, &theme, &mut hits);
    }
    if let Some(rect) = overlay {
        if show_slash {
            render_slash(frame, rect, model, &theme, &mut hits);
        } else if let Some(mentions) = model.mentions.filter(|_| mention_height > 0) {
            crate::conversation::render_mentions(frame, inset(rect), mentions, &theme);
        } else if let Some(body) = model.notice {
            render_notice(frame, rect, body, &theme);
        } else if let Some(body) = model.paste_preview {
            render_notice(frame, rect, body, &theme);
        }
    }
    render_composer(frame, composer, model, &theme, &mut hits);
    render_status(frame, status, model, &theme);
    render_hints(frame, hints, model, &theme, &mut hits);
    if let Some(panel) = model.right_panel {
        panel.draw(frame, model, area);
        hits.side_tool_summary_rows = panel.chat_tool_rows();
    }
    dim_region(
        frame.buffer_mut(),
        terminal_area,
        model.dialog_stack.len() as f64,
        &theme,
    );
    for (index, id) in model.dialog_stack.iter().copied().enumerate() {
        if id == DialogId::Onboarding {
            hits = HitMap {
                modal: true,
                ..HitMap::default()
            };
            continue;
        }
        let mut layer_model = model.clone();
        if index + 1 < model.dialog_stack.len() {
            layer_model.pointer = None;
        }
        let dialog_hits = render_layer(frame, terminal_area, &layer_model, id, &theme);
        if let Some(rect) = dialog_hits.modal_area {
            dim_region(
                frame.buffer_mut(),
                rect,
                (model.dialog_stack.len() - index - 1) as f64,
                &theme,
            );
        }
        hits = dialog_hits;
    }
    hits
}

/// Render each live window before caching only its uncomposited rectangle.
pub fn draw_animated(
    frame: &mut Frame<'_>,
    model: Option<&FrameModel<'_>>,
    transcript_cache: &mut TranscriptCache,
    visuals: &mut crate::dialogs::DialogVisuals,
    now: std::time::Instant,
    mut guide: Option<&mut crate::onboarding::Onboarding>,
    username: &str,
    logged_in: bool,
) -> HitMap {
    let theme = Theme::current();
    let area = frame.area();
    let mut hits = if let Some(model) = model {
        let mut base = model.clone();
        base.dialog_stack = &[];
        draw(frame, &base, transcript_cache)
    } else {
        frame.render_widget(Block::default().style(theme.base()), area);
        frame.render_widget(
            Paragraph::new("终端较小，请扩大窗口；/onboarding 打开引导"),
            area,
        );
        HitMap::default()
    };
    let depth: f64 = visuals.layers.iter().map(|layer| layer.weight(now)).sum();
    dim_region(frame.buffer_mut(), area, depth, &theme);
    let count = visuals.layers.len();
    for index in 0..count {
        let above: f64 = visuals.layers[index + 1..]
            .iter()
            .map(|layer| layer.weight(now))
            .sum();
        let visual = &mut visuals.layers[index];
        let mut layer_hits = HitMap {
            modal: true,
            ..HitMap::default()
        };
        if visual.live() {
            let rect = if visual.id == DialogId::Onboarding {
                guide.as_deref_mut().map(|guide| {
                    guide.draw(frame, username, logged_in);
                    guide.render_area(area)
                })
            } else if let Some(model) = model {
                let mut layer_model = model.clone();
                if index + 1 != count {
                    layer_model.pointer = None;
                }
                layer_hits = render_layer(frame, area, &layer_model, visual.id, &theme);
                layer_hits.modal_area
            } else {
                None
            };
            visual.layer = rect.map(|rect| snapshot_region(frame.buffer_mut(), rect));
            visual.theme = Some(theme);
        }
        if let Some(raw) = &visual.layer {
            let mut layer = raw.clone();
            if let Some(old) = visual.theme {
                retheme_layer(&mut layer, old, theme);
            }
            let rect = layer.area;
            let retain = (0.72_f64.powf(above) * (0.35 + 0.65 * visual.weight(now))).max(0.35);
            blend_region(&mut layer, rect, retain, &theme);
            copy_region(&layer, frame.buffer_mut());
        }
        hits = layer_hits;
    }
    hits
}

fn snapshot_region(source: &ratatui::buffer::Buffer, rect: Rect) -> ratatui::buffer::Buffer {
    let mut layer = ratatui::buffer::Buffer::empty(rect.intersection(source.area));
    copy_region(source, &mut layer);
    layer
}

fn copy_region(source: &ratatui::buffer::Buffer, target: &mut ratatui::buffer::Buffer) {
    let area = source.area.intersection(target.area);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            target[(x, y)] = source[(x, y)].clone();
        }
    }
}

fn retheme_layer(buffer: &mut ratatui::buffer::Buffer, old: Theme, new: Theme) {
    let colors = |t: Theme| {
        [
            t.bg,
            t.bg_raised,
            t.bg_select,
            t.text,
            t.text_dim,
            t.text_mute,
            t.rose,
            t.sage,
            t.amber,
            t.rust,
            t.hairline,
        ]
    };
    let before = colors(old);
    let after = colors(new);
    for cell in &mut buffer.content {
        if let Some(i) = before.iter().position(|color| *color == cell.fg) {
            cell.fg = after[i];
        }
        if let Some(i) = before.iter().position(|color| *color == cell.bg) {
            cell.bg = after[i];
        }
    }
}

fn session_relative_time(
    then: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let seconds = (now - then).num_seconds().max(0);
    let (key, fallback, count) = match seconds {
        0..=9 => ("session.time.now", "刚刚", 0),
        10..=59 => ("session.time.seconds", "{count} 秒前", seconds),
        60..=3599 => ("session.time.minutes", "{count} 分钟前", seconds / 60),
        3600..=86399 => ("session.time.hours", "{count} 小时前", seconds / 3600),
        _ => ("session.time.days", "{count} 天前", seconds / 86400),
    };
    i18n::tr(key, fallback).replace("{count}", &count.to_string())
}

fn render_session_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &SessionPicker,
    sessions: &[SessionSummary],
    pointer: Option<(u16, u16)>,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap {
        modal: true,
        ..HitMap::default()
    };
    let Some(outer) = dialog_outer(
        area,
        110,
        area.height.saturating_sub(2),
        picker.fullscreen,
        picker.minimized,
    ) else {
        return hits;
    };
    let inner = paint_dialog_frame(
        frame,
        outer,
        &i18n::tr("dialog.sessions", "会话"),
        pointer,
        theme,
        &mut hits,
    );
    if picker.minimized || inner.height < 2 {
        return hits;
    }
    let search = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    hits.session_search = Some(search);
    let label = i18n::tr("session.search", "搜索会话");
    let query = if picker.query.is_empty() {
        i18n::tr("session.search.placeholder", "输入标题或内容关键词…")
    } else {
        picker.query.clone()
    };
    frame.render_widget(
        Paragraph::new(ellipsize(
            &format!("{label}: {query}"),
            search.width as usize,
        ))
        .style(theme.fg(theme.text).bg(theme.bg_select)),
        search,
    );
    let footer_height = u16::from(inner.height >= 7);
    let body = Rect::new(
        inner.x,
        search.bottom(),
        inner.width,
        inner.height.saturating_sub(2 + footer_height),
    );
    if footer_height > 0 {
        let hint = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
        frame.render_widget(
            Paragraph::new(ellipsize(
                &i18n::tr(
                    "session.hint",
                    "输入搜索 · ↑/↓ 选择 · Enter 打开 · Esc 关闭",
                ),
                hint.width as usize,
            ))
            .style(theme.mute()),
            hint,
        );
    }
    if body.height == 0 || body.width == 0 {
        return hits;
    }
    let width = body.width as usize;
    let content_count = picker
        .rows
        .iter()
        .filter(|row| row.content.is_some())
        .count();
    let status = if let Some(error) = &picker.error {
        i18n::tr("session.search.error", "内容搜索失败：{error}").replace("{error}", error)
    } else if picker.searching {
        i18n::tr("session.search.searching", "正在深入搜索会话内容…")
    } else {
        i18n::tr("session.search.done", "内容匹配 · {count} 条结果")
            .replace("{count}", &content_count.to_string())
    };
    let needle = picker.query.trim().to_ascii_lowercase();
    let now = chrono::Utc::now();
    let mut lines: Vec<(Option<usize>, Line<'static>)> = vec![(
        None,
        Line::styled(
            ellipsize(&i18n::tr("session.matches", "会话"), width),
            theme.mute(),
        ),
    )];
    let mut content_started = false;
    for (index, row) in picker.rows.iter().enumerate() {
        if row.content.is_some() && !content_started {
            lines.push((
                None,
                Line::styled(
                    ellipsize(&status, width),
                    if picker.error.is_some() {
                        theme.fg(theme.rust)
                    } else {
                        theme.mute()
                    },
                ),
            ));
            content_started = true;
        }
        let Some(session) = sessions.iter().find(|session| session.id == row.id) else {
            continue;
        };
        let title = session
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty() && *title != "tui")
            .map(str::to_owned)
            .unwrap_or_else(|| i18n::tr("session.untitled", "未命名会话"));
        let selected = index == picker.selected;
        let project = Path::new(&session.workspace_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| i18n::tr("session.project.none", "无项目"));
        let metadata = format!(
            "{project} · {}",
            session_relative_time(session.updated_at, now)
        );
        let metadata = ellipsize(&metadata, width.saturating_sub(2) / 2);
        let title_width = width.saturating_sub(2 + metadata.width() + 2);
        let title = ellipsize(&sanitize_title(&title), title_width);
        let gap = width.saturating_sub(2 + title.width() + metadata.width());
        let mut spans = vec![Span::styled(
            if selected { "❯ " } else { "  " },
            theme.fg(theme.rose),
        )];
        spans.extend(highlight_spans(&title, Some(&needle), theme));
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(metadata, theme.mute()));
        lines.push((Some(index), Line::from(spans)));
        let preview = row
            .content
            .as_deref()
            .or_else(|| {
                picker
                    .previews
                    .get(row.id.as_str())
                    .filter(|(sequence, _)| *sequence == session.last_sequence)
                    .map(|(_, text)| text.as_str())
            })
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| i18n::tr("session.preview.empty", "暂无消息预览"));
        let preview = ellipsize(&sanitize_title(&preview), width.saturating_sub(2));
        let mut spans = vec![Span::raw("  ")];
        if row.content.is_some() {
            spans.extend(highlight_spans(&preview, Some(&needle), theme));
        } else {
            spans.push(Span::styled(preview, theme.mute()));
        }
        lines.push((Some(index), Line::from(spans)));
    }
    if !content_started
        && (!picker.query.trim().is_empty() || picker.searching || picker.error.is_some())
    {
        lines.push((None, Line::styled(ellipsize(&status, width), theme.mute())));
    }
    if picker.rows.is_empty() {
        lines.push((
            None,
            Line::styled(
                ellipsize(&i18n::tr("session.matches.empty", "没有匹配的会话"), width),
                theme.mute(),
            ),
        ));
    }
    let capacity = body.height as usize;
    let mut start = picker
        .scroll
        .get()
        .min(lines.len().saturating_sub(capacity));
    if let Some(first) = lines
        .iter()
        .position(|(index, _)| *index == Some(picker.selected))
    {
        let last = (first + 1).min(first + capacity.saturating_sub(1));
        if first < start {
            start = first;
        }
        if last >= start + capacity {
            start = last + 1 - capacity;
        }
    }
    picker.scroll.set(start);
    for (offset, (index, line)) in lines.into_iter().skip(start).take(capacity).enumerate() {
        let rect = Rect::new(body.x, body.y + offset as u16, body.width, 1);
        if let Some(index) = index {
            hits.session_picker_rows.push((rect, index));
            if index == picker.selected {
                frame.render_widget(
                    Block::default().style(Style::default().bg(theme.bg_select)),
                    rect,
                );
            }
        }
        frame.render_widget(Paragraph::new(line), rect);
    }
    hits
}

fn render_layer(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    id: DialogId,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap {
        modal: true,
        ..HitMap::default()
    };
    let mut owned_model = model.clone();
    let control_panel = model.right_panel;
    let side = control_panel.and_then(|panel| panel.control_chat());
    if matches!(
        id,
        DialogId::Provider | DialogId::Permission | DialogId::Context
    ) {
        if let Some(chat) = side {
            owned_model.projection = Some(&chat.projection);
            owned_model.permission_mode = chat.permission(model.permission_mode);
        }
    }
    let model = &owned_model;
    if id == DialogId::Project
        && let Some(picker) = model.project_picker
    {
        let mut projects: Vec<String> = model
            .sessions
            .iter()
            .map(|session| session.workspace_path.clone())
            .filter(|path| !path.trim().is_empty())
            .collect();
        projects.sort();
        projects.dedup();
        let visible = projects.len().min(11) as u16 + 1;
        let menu_width = 72u16.min(area.width.saturating_sub(4));
        let menu_height = (visible + 4).min(area.height.saturating_sub(2));
        let Some(menu_area) = dialog_outer(
            area,
            menu_width,
            menu_height,
            picker.fullscreen,
            picker.minimized,
        ) else {
            return hits;
        };
        let mut dialog_hits = HitMap::default();
        let inner = paint_dialog_frame(
            frame,
            menu_area,
            &i18n::tr("dialog.projects", "项目"),
            model.pointer,
            &theme,
            &mut dialog_hits,
        );
        let [_, body, hint_row] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(2),
            Constraint::Length(1),
        ])
        .areas(inner);
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
        if picker.minimized {
            return hits;
        }
        let rows = split_n_rows(body, visible);
        for row_index in 0..visible as usize {
            if let Some(rect) = rows.get(row_index).copied() {
                hits.project_picker_rows.push((
                    rect,
                    if row_index == visible as usize - 1 {
                        projects.len()
                    } else {
                        row_index
                    },
                ));
                if row_index == picker.selected {
                    frame.render_widget(
                        Block::default().style(Style::default().bg(theme.bg_select)),
                        rect,
                    );
                }
                let marker = if row_index == picker.selected {
                    "❯ "
                } else {
                    "  "
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(marker, theme.fg(theme.rose)),
                        Span::styled(
                            if row_index == visible as usize - 1 {
                                "+ 打开新项目".to_owned()
                            } else {
                                ellipsize(&projects[row_index], 60)
                            },
                            theme.fg(if row_index == visible as usize - 1 {
                                theme.sage
                            } else {
                                theme.text
                            }),
                        ),
                    ]))
                    .style(theme.base()),
                    rect,
                );
            }
        }
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                i18n::tr(
                    "hint.select_confirm_close",
                    "↑/↓ 选择 · Enter 确认 · Esc 关闭",
                ),
                theme.mute(),
            )))
            .style(theme.base()),
            hint_row,
        );
    }
    if id == DialogId::Folder
        && let Some(picker) = model.folder_picker
    {
        let Some(modal) = dialog_outer(area, 76, 18, picker.fullscreen, picker.minimized) else {
            return hits;
        };
        hits.project_picker_rows.clear();
        let mut dialog_hits = HitMap::default();
        let inner = paint_dialog_frame(
            frame,
            modal,
            "打开新项目",
            model.pointer,
            &theme,
            &mut dialog_hits,
        );
        let [_, path_row, body, action, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
        if picker.minimized {
            return hits;
        }
        frame.render_widget(
            Paragraph::new(format!("路径：{}", picker.path.display())).style(theme.dim()),
            path_row,
        );
        let visible = usize::from(body.height);
        let scroll = picker
            .scroll
            .min(picker.entries.len().saturating_sub(visible));
        for (offset, entry) in picker.entries.iter().enumerate().skip(scroll).take(visible) {
            let rect = Rect::new(body.x, body.y + (offset - scroll) as u16, body.width, 1);
            hits.folder_picker_rows.push((rect, offset));
            if offset == picker.selected {
                frame.render_widget(
                    Block::default().style(Style::default().bg(theme.bg_select)),
                    rect,
                );
            }
            let name = if offset == 0 {
                "..".to_owned()
            } else {
                entry
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            frame.render_widget(
                Paragraph::new(format!(
                    "{} {name}/",
                    if offset == picker.selected {
                        "❯"
                    } else {
                        " "
                    }
                ))
                .style(theme.fg(theme.text)),
                rect,
            );
        }
        hits.folder_picker_open = Some(action);
        frame.render_widget(
            Paragraph::new(" + 打开此目录").style(theme.fg(theme.sage)),
            action,
        );
        frame.render_widget(
            Paragraph::new("↑/↓ 选择 · Enter 进入目录 · O 打开项目 · Esc 返回").style(theme.mute()),
            hint,
        );
    }
    if id == DialogId::Session
        && let Some(picker) = model.session_picker
    {
        return render_session_picker(frame, area, picker, model.sessions, model.pointer, theme);
    }

    if id == DialogId::Mode
        && let Some(menu) = model.mode_menu
    {
        let menu_width = 58u16.min(area.width.saturating_sub(4));
        let menu_height = (MODE_OPTIONS.len() as u16 * 3 + 4).min(area.height.saturating_sub(4));
        let Some(menu_area) = dialog_outer(
            area,
            menu_width,
            menu_height,
            menu.fullscreen,
            menu.minimized,
        ) else {
            return hits;
        };
        let mut dialog_hits = HitMap::default();
        let inner = paint_dialog_frame(
            frame,
            menu_area,
            &i18n::tr("dialog.mode", "运行模式"),
            model.pointer,
            &theme,
            &mut dialog_hits,
        );
        let [_, body, hint_row] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(inner);
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
        if menu.minimized {
            return hits;
        }
        let rows = split_n_rows(body, MODE_OPTIONS.len() as u16);
        for (index, option) in MODE_OPTIONS.iter().enumerate() {
            if let Some(rect) = rows.get(index).copied() {
                hits.mode_rows.push((rect, index));
                let selected = index == menu.selected;
                if selected {
                    frame.render_widget(
                        Block::default().style(Style::default().bg(theme.bg_select)),
                        rect,
                    );
                }
                let marker = if selected { "❯ " } else { "  " };
                let line = Line::from(vec![
                    Span::styled(marker, theme.fg(theme.rose)),
                    Span::styled(
                        option.title,
                        theme.fg(theme.text).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  ", theme.mute()),
                    Span::styled(option.description, theme.mute()),
                ]);
                frame.render_widget(Paragraph::new(line).style(theme.base()), rect);
            }
        }
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                i18n::tr(
                    "hint.select_confirm_close",
                    "↑/↓ 选择 · Enter 确认 · Esc 关闭",
                ),
                theme.mute(),
            )))
            .style(theme.base()),
            hint_row,
        );
    }
    if id == DialogId::Git
        && let Some(dialog) = model.git_dialog
    {
        let dialog_hits = render_git_dialog(frame, area, dialog, model.pointer, &theme, model.tick);
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
        hits.git_tabs = dialog_hits.git_tabs;
        hits.git_message = dialog_hits.git_message;
        hits.git_generate = dialog_hits.git_generate;
        hits.git_primary = dialog_hits.git_primary;
        hits.git_files = dialog_hits.git_files;
        hits.git_diff = dialog_hits.git_diff;
        hits.git_rows = dialog_hits.git_rows;
        hits.git_actions = dialog_hits.git_actions;
    } else if id == DialogId::ToolDetail
        && let Some(dialog) = model.tool_detail_dialog
    {
        let dialog_hits = render_tool_detail_dialog(frame, area, dialog, model.pointer, &theme);
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
    } else if id == DialogId::Tool
        && let Some(dialog) = model.tool_dialog
    {
        let dialog_hits = render_tool_dialog(frame, area, dialog, model.pointer, &theme);
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
        hits.tool_detail_rows = dialog_hits.tool_detail_rows;
    } else if id == DialogId::Passport
        && let Some(dialog) = model.passport_dialog
    {
        let dialog_hits = render_passport_dialog(frame, area, dialog, model.pointer, &theme);
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
    } else if id == DialogId::AddProvider
        && let Some(dialog) = model.add_provider_dialog
    {
        let dialog_hits = render_add_provider_dialog(frame, area, dialog, model.pointer, &theme);
        hits.add_provider_rows = dialog_hits.add_provider_rows;
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
    } else if id == DialogId::Theme
        && let Some(dialog) = model.theme_dialog
    {
        let dialog_hits = render_theme_dialog(frame, area, dialog, model.pointer, &theme);
        hits.theme_rows = dialog_hits.theme_rows;
        hits.theme_tabs = dialog_hits.theme_tabs;
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
    } else if id == DialogId::Provider
        && let Some(dialog) = model.provider_dialog
    {
        let dialog_hits = render_provider_dialog(frame, area, dialog, model.pointer, &theme);
        hits.provider_rows = dialog_hits.provider_rows;
        hits.provider_model_rows = dialog_hits.provider_model_rows;
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
    } else if id == DialogId::Context
        && let Some(dialog) = model.context_dialog
    {
        let dialog_hits =
            render_context_dialog(frame, area, model.projection, dialog, model.pointer, &theme);
        hits.context_tabs = dialog_hits.context_tabs;
        hits.context_ranges = dialog_hits.context_ranges;
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
    } else if id == DialogId::Harness
        && let Some(dialog) = model.harness_dialog
    {
        let dialog_hits = render_harness_dialog(frame, area, model, dialog, &theme);
        hits.harness_rows = dialog_hits.harness_rows;
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
    } else if id == DialogId::Permission
        && let Some(dialog) = model.permission_dialog
    {
        let dialog_hits = render_permission_dialog(frame, area, model, dialog, &theme);
        hits.permission_rows = dialog_hits.permission_rows;
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
    } else if id == DialogId::Settings
        && let Some(dialog) = model.settings_dialog
    {
        let dialog_hits = render_settings_dialog(frame, area, model, dialog, &theme);
        hits.settings_rows = dialog_hits.settings_rows;
        hits.settings_tabs = dialog_hits.settings_tabs;
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
    } else if id == DialogId::Question
        && let Some(dialog) = model.question_dialog
    {
        let dialog_hits = render_question_dialog(frame, area, dialog, model.pointer, &theme);
        hits.question_tabs = dialog_hits.question_tabs;
        hits.question_options = dialog_hits.question_options;
        hits.question_rows = dialog_hits.question_rows;
        hits.traffic_lights = dialog_hits.traffic_lights;
        hits.modal_area = dialog_hits.modal_area;
    }
    if matches!(
        id,
        DialogId::Provider | DialogId::Permission | DialogId::Context
    ) {
        if let Some(rect) = hits.modal_area {
            let owner = side
                .map(|chat| format!("{} · {}", i18n::tr("right.chat", "侧聊"), chat.title()))
                .unwrap_or_else(|| i18n::tr("control.main", "主对话"));
            frame.render_widget(
                Paragraph::new(ellipsize(&owner, rect.width.saturating_sub(6) as usize))
                    .style(theme.mute()),
                Rect::new(
                    rect.x + 3,
                    rect.bottom().saturating_sub(1),
                    rect.width.saturating_sub(6),
                    1,
                ),
            );
        }
    }
    hits
}

fn render_question_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    form: &crate::question::QuestionForm,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap {
        modal: true,
        ..HitMap::default()
    };
    let Some(modal) = dialog_outer(area, 78, 22, form.fullscreen, form.minimized) else {
        return hits;
    };
    let inner = paint_dialog_frame(
        frame,
        modal,
        &form
            .owner_label
            .as_ref()
            .map(|owner| format!("{} · {owner}", i18n::tr("question.title", "Blora 提问")))
            .unwrap_or_else(|| i18n::tr("question.title", "Blora 提问")),
        pointer,
        theme,
        &mut hits,
    );
    if form.minimized {
        return hits;
    }
    let [_, tabs, body, hint] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    render_question_tabs(frame, tabs, form, theme, &mut hits);
    if let Some(question) = form.questions.get(form.focus) {
        let index = form.focus;
        let mut row = 0u16;
        if row < body.height {
            frame.render_widget(
                Paragraph::new(ellipsize(&question.question, body.width as usize))
                    .style(theme.dim()),
                Rect::new(body.x, body.y, body.width, 1),
            );
            row += 1;
        }
        let input_reserved = u16::from(form.accepts_text());
        match question.kind.as_str() {
            "single" | "multi" | "boolean" => {
                for (option_index, option) in question.options.iter().enumerate() {
                    if row + input_reserved >= body.height {
                        break;
                    }
                    let rect = Rect::new(body.x, body.y + row, body.width, 1);
                    hits.question_options.push((rect, index, option_index));
                    let selected = if question.kind == "multi" {
                        form.checks[index]
                            .get(option_index)
                            .copied()
                            .unwrap_or(false)
                    } else {
                        form.choice[index] == Some(option_index)
                    };
                    let cursor = form.cursor[index] == option_index;
                    if cursor {
                        frame.render_widget(
                            Block::default().style(Style::default().bg(theme.bg_select)),
                            rect,
                        );
                    }
                    let mark = if question.kind == "multi" {
                        if selected { "[x]" } else { "[ ]" }
                    } else if selected {
                        "(•)"
                    } else {
                        "( )"
                    };
                    let label = if option.other {
                        i18n::tr("question.other", "其他")
                    } else if question.kind == "boolean" && option_index == 0 {
                        i18n::tr("question.yes", "是")
                    } else if question.kind == "boolean" && option_index == 1 {
                        i18n::tr("question.no", "否")
                    } else {
                        option.label.clone()
                    };
                    frame.render_widget(
                        Paragraph::new(format!("  {mark} {label}")).style(theme.fg(theme.text)),
                        rect,
                    );
                    row += 1;
                }
                if form.accepts_text() && row < body.height {
                    paint_question_field(
                        frame,
                        Rect::new(body.x, body.y + row, body.width, 1),
                        &i18n::tr("question.other", "其他"),
                        form.field_text(),
                        theme,
                    );
                }
            }
            "text" => {
                if row < body.height {
                    paint_question_field(
                        frame,
                        Rect::new(body.x, body.y + row, body.width, 1),
                        &i18n::tr("question.field", "回答"),
                        form.field_text(),
                        theme,
                    );
                }
            }
            "folder" | "file" => {
                let visible = body.height.saturating_sub(row).saturating_sub(1);
                let scroll = form.scroll[index].min(
                    form.entries[index]
                        .len()
                        .saturating_sub(usize::from(visible).max(1)),
                );
                for (offset, entry) in form.entries[index]
                    .iter()
                    .enumerate()
                    .skip(scroll)
                    .take(visible as usize)
                {
                    let rect = Rect::new(body.x, body.y + row, body.width, 1);
                    hits.question_rows.push((rect, offset));
                    if offset == form.cursor[index] {
                        frame.render_widget(
                            Block::default().style(Style::default().bg(theme.bg_select)),
                            rect,
                        );
                    }
                    let name = if entry == "*" {
                        i18n::tr("question.pick_folder", "选择此文件夹")
                    } else {
                        entry.clone()
                    };
                    let mark = if form.cursor[index] == offset {
                        "❯"
                    } else {
                        " "
                    };
                    frame.render_widget(
                        Paragraph::new(format!("  {mark} {name}")).style(theme.fg(theme.text)),
                        rect,
                    );
                    row += 1;
                }
                if row < body.height {
                    let picked = form.picked[index].as_deref().unwrap_or("—");
                    frame.render_widget(
                        Paragraph::new(format!(
                            "  {} {picked}",
                            i18n::tr("question.picked", "已选")
                        ))
                        .style(theme.dim()),
                        Rect::new(body.x, body.y + row, body.width, 1),
                    );
                }
            }
            _ => {}
        }
    }
    let ready = if form.formatted().is_some() {
        i18n::tr("question.ready", "Enter 提交")
    } else {
        i18n::tr("question.pending", "还有题目未回答")
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{} · {ready}",
            i18n::tr(
                "question.hint",
                "← → 切换 · ↑ ↓ 选择或换题 · Space 确定 · Esc 跳过"
            )
        ))
        .style(theme.mute()),
        hint,
    );
    hits
}

fn render_question_tabs(
    frame: &mut Frame<'_>,
    tabs: Rect,
    form: &crate::question::QuestionForm,
    theme: &Theme,
    hits: &mut HitMap,
) {
    if tabs.width == 0 || tabs.height == 0 || form.questions.is_empty() {
        return;
    }
    let labels: Vec<String> = form
        .questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let mark = if index == form.focus {
                "▸"
            } else if form.is_answered(index) {
                "✓"
            } else {
                "·"
            };
            format!(" {mark} {} ", question.answer_title())
        })
        .collect();
    let widths: Vec<u16> = labels
        .iter()
        .map(|label| (label.width() as u16).max(1))
        .collect();
    let total: u16 = widths.iter().copied().sum();
    let overflow = total > tabs.width && tabs.width > 2;
    let gutter = u16::from(overflow);
    let view = tabs.width.saturating_sub(gutter.saturating_mul(2));
    let focus = form.focus.min(form.questions.len() - 1);
    let scroll = if overflow {
        question_tab_scroll(&widths, focus, view)
    } else {
        0
    };
    let chrome = theme.mute().bg(theme.bg);
    if overflow {
        frame.render_widget(
            Paragraph::new(if scroll > 0 { "‹" } else { " " }).style(chrome),
            Rect::new(tabs.x, tabs.y, 1, 1),
        );
        let more = scroll.saturating_add(view) < total;
        frame.render_widget(
            Paragraph::new(if more { "›" } else { " " }).style(chrome),
            Rect::new(tabs.x + tabs.width - 1, tabs.y, 1, 1),
        );
    }
    let origin = tabs.x + gutter;
    let mut col = 0u16;
    for (index, (label, width)) in labels.iter().zip(&widths).enumerate() {
        let tab_start = col;
        let tab_end = col.saturating_add(*width);
        col = tab_end;
        if tab_end <= scroll || tab_start >= scroll.saturating_add(view) {
            continue;
        }
        let visible_start = tab_start.max(scroll);
        let visible_end = tab_end.min(scroll.saturating_add(view));
        let rect = Rect::new(
            origin + (visible_start - scroll),
            tabs.y,
            visible_end - visible_start,
            1,
        );
        hits.question_tabs.push((rect, index));
        let style = if index == focus {
            theme
                .fg(theme.text)
                .bg(theme.bg_select)
                .add_modifier(Modifier::BOLD)
        } else if form.is_answered(index) {
            theme.fg(theme.sage).bg(theme.bg)
        } else {
            theme.mute().bg(theme.bg)
        };
        frame.render_widget(
            Paragraph::new(slice_cols(
                label,
                usize::from(visible_start - tab_start),
                usize::from(visible_end - visible_start),
            ))
            .style(style),
            rect,
        );
    }
}

fn question_tab_scroll(widths: &[u16], focus: usize, view: u16) -> u16 {
    if view == 0 {
        return 0;
    }
    let mut acc = 0u16;
    for (index, width) in widths.iter().enumerate() {
        if index == focus {
            let end = acc.saturating_add(*width);
            return if end > view {
                end.saturating_sub(view)
            } else {
                0
            };
        }
        acc = acc.saturating_add(*width);
    }
    0
}

fn paint_question_field(
    frame: &mut Frame<'_>,
    rect: Rect,
    label: &str,
    value: &str,
    theme: &Theme,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let prefix = format!(" {label} [");
    let cursor = "▌";
    let close = "]";
    let placeholder = i18n::tr("question.placeholder", "在此输入…");
    let empty = value.is_empty();
    let budget = usize::from(rect.width)
        .saturating_sub(prefix.width() + cursor.width().max(1) + close.width());
    let body = tail_cols(if empty { placeholder.as_str() } else { value }, budget);
    let base = Style::default().bg(theme.bg_select).fg(theme.text);
    let line = Line::from(vec![
        Span::styled(prefix, base.fg(theme.text_dim)),
        Span::styled(
            body,
            if empty {
                base.fg(theme.text_mute)
            } else {
                base
            },
        ),
        Span::styled(
            cursor.to_owned(),
            base.fg(theme.rose).add_modifier(Modifier::BOLD),
        ),
        Span::styled(close.to_owned(), base.fg(theme.text_dim)),
    ]);
    frame.render_widget(Paragraph::new(line).style(base), rect);
}

fn slice_cols(text: &str, skip: usize, take: usize) -> String {
    if take == 0 {
        return String::new();
    }
    let mut seen = 0usize;
    let mut taken = 0usize;
    let mut out = String::new();
    for ch in text.chars() {
        let width = ch.width().unwrap_or(0);
        if seen < skip {
            seen += width;
            continue;
        }
        if taken + width > take {
            break;
        }
        out.push(ch);
        taken += width;
    }
    out
}

fn tail_cols(text: &str, max: usize) -> String {
    if max == 0 || text.is_empty() {
        return String::new();
    }
    if text.width() <= max {
        return text.to_owned();
    }
    let mut acc = 0usize;
    let mut start = text.len();
    for (idx, ch) in text.char_indices().rev() {
        let width = ch.width().unwrap_or(0);
        if acc + width > max {
            break;
        }
        acc += width;
        start = idx;
    }
    text[start..].to_owned()
}

fn color_rgb(color: Color, fallback: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset => {
            if fallback == Color::Reset {
                (128, 128, 128)
            } else {
                color_rgb(fallback, Color::Reset)
            }
        }
        Color::Indexed(index) if index >= 232 => {
            let value = 8 + (index - 232) * 10;
            (value, value, value)
        }
        Color::Indexed(index) if index >= 16 => {
            let index = index - 16;
            let channel = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            (
                channel(index / 36),
                channel(index / 6 % 6),
                channel(index % 6),
            )
        }
        _ => {
            let index = match color {
                Color::Black => 0,
                Color::Red => 1,
                Color::Green => 2,
                Color::Yellow => 3,
                Color::Blue => 4,
                Color::Magenta => 5,
                Color::Cyan => 6,
                Color::Gray => 7,
                Color::DarkGray => 8,
                Color::LightRed => 9,
                Color::LightGreen => 10,
                Color::LightYellow => 11,
                Color::LightBlue => 12,
                Color::LightMagenta => 13,
                Color::LightCyan => 14,
                Color::White => 15,
                Color::Indexed(n) => usize::from(n),
                _ => unreachable!(),
            };
            const ANSI: [(u8, u8, u8); 16] = [
                (0, 0, 0),
                (128, 0, 0),
                (0, 128, 0),
                (128, 128, 0),
                (0, 0, 128),
                (128, 0, 128),
                (0, 128, 128),
                (192, 192, 192),
                (128, 128, 128),
                (255, 0, 0),
                (0, 255, 0),
                (255, 255, 0),
                (0, 0, 255),
                (255, 0, 255),
                (0, 255, 255),
                (255, 255, 255),
            ];
            ANSI[index]
        }
    }
}

fn dim_region(buffer: &mut ratatui::buffer::Buffer, area: Rect, depth: f64, theme: &Theme) {
    if depth == 0.0 {
        return;
    }
    // Compute from the freshly rendered layer to keep redraws and closing reversible.
    let retain = 0.72_f64.powf(depth.max(0.0)).max(0.35);
    blend_region(buffer, area, retain, theme);
}

fn blend_region(buffer: &mut ratatui::buffer::Buffer, area: Rect, retain: f64, theme: &Theme) {
    if retain == 1.0 {
        return;
    }
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let Some(cell) = buffer.cell_mut((x, y)) else {
                continue;
            };
            if cell.modifier.contains(Modifier::REVERSED) {
                let fg = cell.fg;
                cell.fg = cell.bg;
                cell.bg = fg;
                cell.modifier.remove(Modifier::REVERSED);
            }
            let default_fg = if theme.text == Color::Reset {
                Color::Rgb(32, 32, 32)
            } else {
                theme.text
            };
            let default_bg = if theme.bg == Color::Reset {
                Color::Rgb(250, 250, 250)
            } else {
                theme.bg
            };
            let fg = color_rgb(cell.fg, default_fg);
            let bg = color_rgb(cell.bg, default_bg);
            let blend = |f: u8, b: u8| {
                (f64::from(b) + (f64::from(f) - f64::from(b)) * retain).round() as u8
            };
            cell.fg = Color::Rgb(blend(fg.0, bg.0), blend(fg.1, bg.1), blend(fg.2, bg.2));
            cell.modifier.remove(
                Modifier::BOLD | Modifier::DIM | Modifier::SLOW_BLINK | Modifier::RAPID_BLINK,
            );
        }
    }
}

fn inset(area: Rect) -> Rect {
    let pad = if area.width >= 60 {
        PAD
    } else {
        area.width.min(2) / 2
    };
    Rect {
        x: area.x.saturating_add(pad),
        y: area.y,
        width: area.width.saturating_sub(pad.saturating_mul(2)),
        height: area.height,
    }
}

/// Centered dialog frame. Yellow minimizes to a title bar; green fills the
/// terminal minus a one-cell margin.
pub(super) fn dialog_outer(
    area: Rect,
    compact_width: u16,
    compact_height: u16,
    fullscreen: bool,
    minimized: bool,
) -> Option<Rect> {
    if area.width < 20 || area.height < 3 {
        return None;
    }
    let height = if minimized {
        3.min(area.height)
    } else if fullscreen {
        area.height.saturating_sub(2).max(5).min(area.height)
    } else {
        compact_height.min(area.height.saturating_sub(2)).max(5)
    };
    let width = if fullscreen {
        area.width.saturating_sub(2).max(20).min(area.width)
    } else {
        compact_width.clamp(24, area.width.saturating_sub(4).max(24))
    };
    let [_, frame_x, _] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(width.min(area.width)),
        Constraint::Min(0),
    ])
    .flex(Flex::Center)
    .areas(area);
    let [_, frame_y, _] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(height.min(area.height)),
        Constraint::Min(0),
    ])
    .flex(Flex::Center)
    .areas(frame_x);
    Some(frame_y)
}

/// Rounded dialog chrome. Border cells must use the canvas background;
/// `border_style` with only a foreground would reset the cell bg to the
/// terminal default and show as a white ring.
fn paint_dialog_chrome(frame: &mut Frame<'_>, area: Rect, theme: &Theme) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(theme.base())
        .border_style(theme.fg(theme.hairline).bg(theme.bg));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Draw the common dialog border and title row, including clickable window controls.
pub(super) fn paint_dialog_frame(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
    hits: &mut HitMap,
) -> Rect {
    hits.modal_area = Some(area);
    let inner = paint_dialog_chrome(frame, area, theme);
    let title_row = Rect::new(inner.x, inner.y, inner.width, inner.height.min(1));
    paint_traffic_title(frame, title_row, title, pointer, theme, hits);
    inner
}

/// Paint macOS-style traffic lights and register their hit rects. Hovering a
/// dot swaps in ✕ / − / +.
fn paint_traffic_title(
    frame: &mut Frame<'_>,
    title_row: Rect,
    title: &str,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let dot_glyphs: [(&str, &str, Color); 3] = [
        ("●", "✕", theme.rust),
        ("●", "−", theme.amber),
        ("●", "+", theme.sage),
    ];
    for (index, _) in dot_glyphs.iter().enumerate() {
        // Use a two-cell hit target so the dots remain clickable with terminal
        // mouse coordinate rounding and narrow glyph metrics.
        hits.traffic_lights[index] = Some(Rect {
            x: title_row.x.saturating_add(1 + (index as u16) * 2),
            y: title_row.y,
            width: 2,
            height: 1,
        });
    }
    let hovered_dot = pointer.and_then(|(col, row)| {
        hits.traffic_lights
            .iter()
            .position(|rect| rect.is_some_and(|rect| contains(rect, col, row)))
    });
    let mut title_line = vec![Span::styled(" ", theme.base())];
    for (index, (dot, symbol, color)) in dot_glyphs.iter().enumerate() {
        let glyph = if hovered_dot == Some(index) {
            symbol
        } else {
            dot
        };
        if index > 0 {
            title_line.push(Span::styled(" ", theme.base()));
        }
        title_line.push(Span::styled((*glyph).to_owned(), theme.fg(*color)));
    }
    title_line.push(Span::styled("  ".to_owned(), theme.mute()));
    title_line.push(Span::styled(title.to_owned(), theme.fg(theme.text_dim)));
    let title_span_width = 1 + " ● ● ●".width() + "  ".width() + title.width();
    if title_span_width >= usize::from(title_row.width) {
        title_line.truncate(6);
    }
    frame.render_widget(
        Paragraph::new(Line::from(title_line)).style(theme.base()),
        title_row,
    );
}

fn git_indicator_text(status: &GitStatusInfo) -> String {
    if status.clean {
        format!("↑{} ↓{} ⍟{}", status.ahead, status.behind, status.stashes)
    } else {
        format!("+{}-{}", status.added, status.removed)
    }
}

fn git_header_rect(inner: Rect, context: Rect, text_width: usize) -> Option<Rect> {
    let width = u16::try_from(text_width).ok()?;
    let x = context.x.checked_sub(width.checked_add(2)?)?;
    (x >= inner.x && width > 0).then_some(Rect {
        x,
        y: inner.y,
        width,
        height: 1,
    })
}

/// Add a header item only inside its allocated columns, including its hit area.
fn header_item(
    line: &mut Line<'static>,
    area: Rect,
    separator: &str,
    text: &str,
    style: Style,
    truncate: bool,
) -> Option<Rect> {
    let offset = line.width();
    let remaining = usize::from(area.width).saturating_sub(offset);
    let separator = if offset == 0 { "" } else { separator };
    let available = remaining.saturating_sub(separator.width());
    if available == 0 || text.is_empty() || (!truncate && text.width() > available) {
        return None;
    }
    let text = ellipsize(text, available);
    line.spans.push(Span::styled(separator.to_owned(), style));
    let rect = Rect::new(
        area.x + (offset + separator.width()) as u16,
        area.y,
        text.width() as u16,
        1,
    );
    line.spans.push(Span::styled(text, style));
    Some(rect)
}

fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let inner = inset(area);
    let session = model
        .projection
        .and_then(|projection| projection.session.as_ref());
    let mode = session.map(|item| item.mode.as_str()).unwrap_or("code");
    let active_workspace = model
        .sessions
        .get(model.index)
        .map(|session| Path::new(&session.workspace_path))
        .unwrap_or(model.workspace);
    let project_name = active_workspace
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("项目");
    let title = session
        .and_then(|item| item.title.clone())
        .filter(|text| !text.is_empty());
    let display_title = title.as_deref().unwrap_or("");
    let mode_session_indices: Vec<usize> = model
        .sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| session.mode.as_str() == mode)
        .map(|(index, _)| index)
        .collect();
    let mode_position = mode_session_indices
        .iter()
        .position(|index| *index == model.index)
        .map(|position| position + 1)
        .unwrap_or(1);
    let mode_count = mode_session_indices.len().max(1);
    let counter = format!("{mode_position}/{mode_count}");
    let mode_title = mode_label(mode);
    let compact = inner.width < 80;
    let separator = if compact { " · " } else { "  ·  " };
    let compactions = model.projection.map_or(0, |p| p.compaction_count);
    let compaction_badge = (compactions > 0).then(|| {
        let color = if compactions >= blora_context::COMPACTION_WARNING {
            theme.amber
        } else {
            theme.text_dim
        };
        Span::styled(format!("⟲{compactions} "), theme.fg(color))
    });
    let right = if model.running {
        let mut line = Line::from(Span::styled(
            if compact {
                spinner(model.tick).to_string()
            } else {
                format!("{}  running", spinner(model.tick))
            },
            theme.fg(theme.sage),
        ));
        if let Some(badge) = compaction_badge.clone() {
            line.spans.insert(0, badge);
        }
        line
    } else {
        let used = model
            .projection
            .map(|projection| {
                projection
                    .input_tokens
                    .saturating_add(projection.output_tokens)
            })
            .unwrap_or(0);
        let window = blora_context::context_window_for(blora_catalog::context_limit(model.model));
        let mut usage = context_usage_line(used, window, theme);
        if let Some(badge) = compaction_badge {
            usage.spans.insert(0, badge);
        }
        if compact {
            Line::from(Span::styled(
                usage.spans.last().unwrap().content.trim().to_owned(),
                theme.mute(),
            ))
        } else {
            usage
        }
    };
    // Keep the harness entry reachable before allocating status columns.
    let width = if inner.width >= 20 {
        right.width() as u16
    } else {
        0
    };
    let rect = Rect {
        x: inner.x + inner.width.saturating_sub(width),
        y: inner.y,
        width,
        height: 1,
    };
    let left_area = Rect::new(
        inner.x,
        inner.y,
        inner.width.saturating_sub(width + u16::from(width > 0)),
        1,
    );
    let mut left = Line::default();
    let harness = model.projection.map(|p| p.harness).unwrap_or_default();
    hits.harness = header_item(
        &mut left,
        left_area,
        separator,
        harness.display_name(),
        theme.rose_bold(),
        true,
    );
    if inner.width < 60 {
        hits.session_picker = header_item(
            &mut left,
            left_area,
            separator,
            &i18n::tr("dialog.sessions", "会话"),
            theme.fg(theme.text_dim),
            false,
        );
    }
    hits.mode = header_item(
        &mut left,
        left_area,
        separator,
        &mode_title,
        theme.fg(theme.sage),
        true,
    );
    let navigation = format!("‹ {counter} ›");
    if let Some(nav) = header_item(
        &mut left,
        left_area,
        separator,
        &navigation,
        theme.dim(),
        false,
    ) {
        hits.prev_session = Some(Rect::new(nav.x, nav.y, 1, 1));
        hits.next_session = Some(Rect::new(nav.right() - 1, nav.y, 1, 1));
    }
    hits.project_picker = header_item(
        &mut left,
        left_area,
        separator,
        project_name,
        theme.fg(theme.sage),
        true,
    );
    if hits.session_picker.is_none() {
        hits.session_picker = header_item(
            &mut left,
            left_area,
            separator,
            &ellipsize(display_title, 28),
            theme.fg(theme.text_dim),
            true,
        );
    }
    frame.render_widget(Paragraph::new(left.clone()).style(theme.base()), left_area);
    if width > 0 {
        if model.running {
            hits.cancel_run = Some(rect);
        } else {
            hits.context_usage = Some(rect);
        }
        frame.render_widget(Paragraph::new(right).style(theme.base()), rect);
    }
    if !compact {
        if let Some(git_text) = model.git_status.map(git_indicator_text) {
            if let Some(git_rect) = git_header_rect(inner, rect, git_text.width()) {
                if usize::from(git_rect.x - inner.x) >= left.width() + 2 {
                    hits.git_status = Some(git_rect);
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(git_text, theme.fg(theme.sage))))
                            .style(theme.base()),
                        git_rect,
                    );
                }
            }
        }
    }
}

fn render_harness_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    dialog: &HarnessDialog,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap::default();
    let Some(modal) = dialog_outer(area, 80, 18, dialog.fullscreen, dialog.minimized) else {
        return hits;
    };
    let inner = paint_dialog_frame(
        frame,
        modal,
        &i18n::tr("harness.title", "执行内核"),
        model.pointer,
        theme,
        &mut hits,
    );
    let inner = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(1),
    );
    if dialog.minimized || inner.height == 0 {
        return hits;
    }
    let kinds = blora_types::HarnessKind::all();
    let extra_rows = if inner.height as usize > kinds.len() {
        3
    } else {
        0
    };
    let capacity = (inner.height.saturating_sub(extra_rows) as usize).min(kinds.len());
    let start = dialog
        .selected
        .saturating_sub(capacity.saturating_sub(1))
        .min(kinds.len().saturating_sub(capacity));
    let current = model.projection.map(|p| p.harness).unwrap_or_default();
    for (index, kind) in kinds.iter().enumerate().skip(start).take(capacity) {
        let row = Rect::new(inner.x, inner.y + (index - start) as u16, inner.width, 1);
        let available = dialog.available.get(index).copied().unwrap_or(false);
        let state = if !available {
            i18n::tr("harness.missing", "未安装")
        } else if *kind == current {
            i18n::tr("harness.current", "当前")
        } else {
            i18n::tr("harness.installed", "已安装")
        };
        let core = format!(
            "{}{}",
            if index == dialog.selected {
                "› "
            } else {
                "  "
            },
            kind.display_name()
        );
        let full = format!("{core} · {state}");
        let text = if usize::from(row.width) >= full.width() {
            full
        } else {
            core
        };
        let style = if !available {
            theme.mute()
        } else if index == dialog.selected {
            theme.rose_bold().bg(theme.bg_select)
        } else {
            theme.base()
        };
        frame.render_widget(
            Paragraph::new(ellipsize(&text, row.width as usize)).style(style),
            row,
        );
        hits.harness_rows.push((row, index));
    }
    if inner.height as usize > capacity {
        frame.render_widget(Paragraph::new(i18n::tr("harness.boundary", "仅前台会话使用所选内核。外部 CLI 使用自身登录、模型、规则、插件和权限/沙盒；不受 Blora 原生配置控制。后台任务、Git 辅助和 Imagine 保持 Blora 原生执行。运行中禁止切换；未安装项不可选。" )).wrap(ratatui::widgets::Wrap { trim: false }).style(theme.dim()), Rect::new(inner.x, inner.y + capacity as u16, inner.width, inner.height.saturating_sub(capacity as u16 + 1)));
        frame.render_widget(
            Paragraph::new(i18n::tr(
                "hint.select_confirm_close",
                "↑/↓ 选择 · Enter 确认 · Esc 关闭",
            ))
            .style(theme.mute()),
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        );
    }
    hits
}

fn render_permission_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    dialog: &PermissionDialog,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap::default();
    let Some(modal) = dialog_outer(area, 76, 14, dialog.fullscreen, dialog.minimized) else {
        return hits;
    };
    let inner = paint_dialog_frame(
        frame,
        modal,
        &i18n::tr("dialog.permission", "权限模式"),
        model.pointer,
        theme,
        &mut hits,
    );
    let inner = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(1),
    );
    if dialog.minimized || inner.height == 0 {
        return hits;
    }
    if dialog.confirm_bypass {
        frame.render_widget(
            Paragraph::new(if model.projection.is_some_and(|p| p.harness != blora_types::HarnessKind::Blora) { i18n::tr("harness.bypass_warning", "风险：跳过外部 CLI 权限确认可能删除文件、泄露密钥或执行危险命令。Blora 原生网络、路径和沙盒约束不覆盖外部 CLI。仅在可信工作区启用。

y 明确启用 · n / Esc 取消（Enter 不会启用）") } else { i18n::tr("permission.bypass.warning", "风险：跳过权限确认可能删除文件、泄露密钥或执行危险命令。\n仅在可信工作区使用；网络禁用与沙盒约束仍有效。\n\ny 明确启用 · n / Esc 取消（Enter 不会启用）") })
                .wrap(ratatui::widgets::Wrap { trim: false })
                .style(theme.fg(theme.rust)),
            Rect::new(inner.x, inner.y, inner.width, inner.height.saturating_sub(1)),
        );
        frame.render_widget(
            Paragraph::new(i18n::tr(
                "permission.bypass.confirm_hint",
                "y 启用 · n/Esc 取消",
            ))
            .style(theme.fg(theme.rust)),
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        );
        return hits;
    }
    let compact = inner.height < 10;
    let row_height = if compact { 1 } else { 2 };
    let capacity = (inner.height.saturating_sub(2) / row_height).max(1) as usize;
    let start = dialog.selected.saturating_sub(capacity.saturating_sub(1));
    for (index, mode) in PermissionMode::ALL
        .iter()
        .enumerate()
        .skip(start)
        .take(capacity)
    {
        let rect = Rect::new(
            inner.x,
            inner.y + ((index - start) as u16 * row_height),
            inner.width,
            row_height,
        );
        if rect.bottom() > inner.bottom() {
            break;
        }
        let name = i18n::permission_name(*mode);
        let current = if *mode == model.permission_mode {
            format!(" · {}", i18n::tr("permission.current", "当前"))
        } else {
            String::new()
        };
        let title = if usize::from(inner.width) < name.width() + 2 {
            name
        } else {
            format!(
                "{} {}{}",
                if index == dialog.selected { "›" } else { " " },
                name,
                current
            )
        };
        let text = if compact {
            title
        } else {
            format!("{title}\n  {}", permission_description(*mode))
        };
        let style = if index == dialog.selected {
            theme.base().bg(theme.bg_select)
        } else {
            theme.base()
        };
        frame.render_widget(Paragraph::new(text).style(style), rect);
        hits.permission_rows.push((rect, index));
    }
    if inner.height >= 2 {
        let detail = Rect::new(inner.x, inner.bottom() - 2, inner.width, 1);
        frame.render_widget(
            Paragraph::new(if model.projection.is_some_and(|p| p.harness != blora_types::HarnessKind::Blora) { i18n::tr("harness.permission_scope", "外部内核使用自身权限和沙盒。此选择仅映射到 CLI 支持的模式；不支持的组合会被拒绝，Blora 原生边界不覆盖外部工具。") } else { permission_description(PermissionMode::ALL[dialog.selected]) })
                .style(theme.mute()),
            detail,
        );
        let hint = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
        frame.render_widget(
            Paragraph::new(i18n::tr(
                "permission.select_hint",
                "↑↓ 选择 · Enter 确认 · Esc 取消",
            ))
            .style(theme.mute()),
            hint,
        );
    }
    hits
}

fn render_settings_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    dialog: &SettingsDialog,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap::default();
    let Some(modal) = dialog_outer(area, 76, 15, dialog.fullscreen, dialog.minimized) else {
        return hits;
    };
    let inner = paint_dialog_frame(frame, modal, "设置", model.pointer, theme, &mut hits);
    if dialog.minimized {
        return hits;
    }
    let [_, tabs, body, hint] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(inner);
    let native_settings = i18n::tr("harness.native_settings", "Blora 原生设置");
    let pages = ["常规", "外观", native_settings.as_str()];
    let tab_width = (tabs.width / pages.len() as u16).max(1);
    for (index, label) in pages.iter().enumerate() {
        let rect = Rect {
            x: tabs.x + tab_width * index as u16,
            y: tabs.y,
            width: if index + 1 == pages.len() {
                tabs.width.saturating_sub(tab_width * index as u16)
            } else {
                tab_width
            },
            height: tabs.height,
        };
        hits.settings_tabs.push((rect, index));
        frame.render_widget(
            Paragraph::new(if index == dialog.page {
                format!("[ {label} ]")
            } else {
                format!("  {label}  ")
            })
            .style(if index == dialog.page {
                theme.fg(theme.rose).add_modifier(Modifier::BOLD)
            } else {
                theme.mute()
            }),
            rect,
        );
    }
    if dialog.minimized {
        return hits;
    }
    let permission_name = i18n::permission_name(model.permission_mode);
    let permission_label = i18n::tr("dialog.permission", "权限模式");
    let rows: &[(&str, &str)] = match dialog.page {
        0 => &[
            (&permission_label, &permission_name),
            (
                "隐藏工具调用",
                if model.hide_tools { "开启" } else { "关闭" },
            ),
            ("确认提示", "始终显示"),
            ("终端标题", "启用"),
            ("首次使用引导", "打开"),
        ],
        1 => &[
            ("主题", model.theme_name()),
            ("配色模式", theme::scheme().as_str()),
            ("动画效果", "启用"),
            ("界面语言", "简体中文"),
            (
                "消息角色标记",
                if model.message_roles {
                    "开启"
                } else {
                    "关闭"
                },
            ),
        ],
        _ => &[
            (
                "供应商",
                if model.provider.is_empty() {
                    "默认"
                } else {
                    model.provider
                },
            ),
            (
                "模型",
                if model.model.is_empty() {
                    "默认"
                } else {
                    model.model
                },
            ),
            ("上下文窗口", "自动"),
            ("响应方式", "流式"),
        ],
    };
    let rows_area = split_n_rows(body, rows.len() as u16);
    for (index, (label, value)) in rows.iter().enumerate() {
        let rect = rows_area[index];
        let selected = index == dialog.selected;
        let style = if selected {
            theme.base().bg(theme.bg_select)
        } else {
            theme.base()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(if selected { "❯ " } else { "  " }, theme.fg(theme.rose)),
                Span::styled(format!("{label:<12}"), theme.fg(theme.text)),
                Span::styled(*value, theme.fg(theme.sage)),
            ]))
            .style(style),
            rect,
        );
        hits.settings_rows.push((rect, index));
    }
    frame.render_widget(
        Paragraph::new("↑↓ 选择 · ←→ 修改 · Enter 确认 · Esc 关闭").style(theme.mute()),
        hint,
    );
    hits
}

fn render_context_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: Option<&SessionProjection>,
    dialog: &ContextDialog,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap::default();
    let Some(modal) = dialog_outer(area, 100, 26, dialog.fullscreen, dialog.minimized) else {
        return hits;
    };
    let inner = paint_dialog_frame(
        frame,
        modal,
        &i18n::tr("dialog.context_usage", "上下文用量"),
        pointer,
        theme,
        &mut hits,
    );
    let [_, tabs, content, hint_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(inner);
    if dialog.minimized {
        return hits;
    }
    paint_context_tabs(frame, tabs, dialog.page, theme, &mut hits);
    let today = chrono::Utc::now().date_naive();
    let summary = crate::usage_stats::summarize(&dialog.activity, dialog.range, today);
    let rendered = match dialog.page {
        0 => crate::usage_stats::RenderedStats {
            lines: context_current_lines(projection, content.width as usize, theme),
            range_line: None,
            range_spans: [(0, 0); 3],
        },
        1 => crate::usage_stats::overview(&summary, content.width as usize, theme),
        _ => crate::usage_stats::models(&summary, content.width as usize, theme),
    };
    place_range_hits(&mut hits, content, &rendered, dialog.scroll);
    let mut lines = rendered.lines;
    let visible = lines.len().saturating_sub(content.height as usize);
    let start = dialog.scroll.min(visible);
    if content.height > 0 {
        lines.drain(0..start);
    }
    frame.render_widget(Paragraph::new(lines).style(theme.base()), content);
    frame.render_widget(
        Paragraph::new(crate::usage_stats::hint(dialog.page)).style(theme.mute()),
        hint_row,
    );
    hits
}

fn paint_context_tabs(
    frame: &mut Frame<'_>,
    tabs: Rect,
    page: usize,
    theme: &Theme,
    hits: &mut HitMap,
) {
    if tabs.width == 0 || tabs.height == 0 {
        return;
    }
    for index in 0..3 {
        let start = tabs.x + tabs.width * u16::try_from(index).unwrap_or(0) / 3;
        let end = tabs.x + tabs.width * u16::try_from(index + 1).unwrap_or(3) / 3;
        let rect = Rect::new(start, tabs.y, end.saturating_sub(start).max(1), 1);
        hits.context_tabs.push((rect, index));
        let selected = page == index;
        let style = if selected {
            theme
                .fg(theme.text)
                .bg(theme.bg_select)
                .add_modifier(Modifier::BOLD)
        } else {
            theme.mute()
        };
        frame.render_widget(
            Paragraph::new(format!(
                " {} {} ",
                if selected { "▸" } else { " " },
                crate::usage_stats::page_label(index)
            ))
            .style(style),
            rect,
        );
    }
}

fn place_range_hits(
    hits: &mut HitMap,
    content: Rect,
    rendered: &crate::usage_stats::RenderedStats,
    scroll: usize,
) {
    let Some(range_line) = rendered.range_line else {
        return;
    };
    let start = scroll.min(rendered.lines.len().saturating_sub(content.height as usize));
    let visible = range_line as i64 - start as i64;
    if visible < 0 || visible >= i64::from(content.height) {
        return;
    }
    let y = content.y + u16::try_from(visible).unwrap_or(0);
    for (index, (column, span_width)) in rendered.range_spans.iter().enumerate() {
        if *span_width == 0 {
            continue;
        }
        let x = content
            .x
            .saturating_add(u16::try_from(*column).unwrap_or(0));
        let room = content.width.saturating_sub(x.saturating_sub(content.x));
        let width = u16::try_from(*span_width).unwrap_or(0).min(room);
        if width > 0 {
            hits.context_ranges.push((Rect::new(x, y, width, 1), index));
        }
    }
}

fn context_current_lines(
    projection: Option<&SessionProjection>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let Some(projection) = projection else {
        return vec![Line::from(Span::styled(
            i18n::tr("context.no_session", "当前没有打开的会话。"),
            theme.mute(),
        ))];
    };
    let window = blora_context::context_window().max(1);
    let used = projection
        .input_tokens
        .saturating_add(projection.output_tokens);
    let input = projection.input_tokens.min(used);
    let output = projection.output_tokens.min(used.saturating_sub(input));
    let system = estimate_system_tokens(projection).min(input);
    let messages = input.saturating_sub(system);
    let free = window.saturating_sub(used);
    let used_pct = percentage(used, window);
    vec![
        Line::from(vec![
            Span::styled(
                format!("{}  ", i18n::tr("dialog.context_current", "当前上下文")),
                theme.fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "{} / {} tokens ({used_pct}%)",
                    fmt_tokens(used),
                    fmt_tokens(window)
                ),
                theme.fg(theme.sage),
            ),
        ]),
        Line::from(Span::styled(
            i18n::tr("dialog.context_total_progress", "总量进度"),
            theme.mute(),
        )),
        progress_line(
            &[
                (system, theme.rose),
                (messages, theme.sage),
                (output, theme.amber),
                (free, theme.text_mute),
            ],
            window,
            width.max(10),
            theme,
        ),
        Line::default(),
        usage_row(
            &i18n::tr("context.system_prompt", "系统提示"),
            system,
            window,
            theme.rose,
            theme,
        ),
        usage_row(
            &i18n::tr("context.messages", "消息内容"),
            messages,
            window,
            theme.sage,
            theme,
        ),
        usage_row(
            &i18n::tr("context.output_tools", "输出与工具"),
            output,
            window,
            theme.amber,
            theme,
        ),
        usage_row(
            &i18n::tr("context.free", "可用空间"),
            free,
            window,
            theme.text_mute,
            theme,
        ),
        Line::default(),
        Line::from(Span::styled(
            i18n::tr(
                "context.estimate_note",
                "数据来自当前会话的累计 token 使用量；百分比按上下文窗口估算。",
            ),
            theme.mute(),
        )),
    ]
}

fn estimate_system_tokens(projection: &SessionProjection) -> u64 {
    projection
        .transcript
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::System { summary, .. } => Some(blora_context::estimate_tokens(summary)),
            _ => None,
        })
        .sum()
}

fn percentage(value: u64, total: u64) -> u64 {
    ((value as f64 / total.max(1) as f64) * 100.0).round() as u64
}

fn progress_line(
    parts: &[(u64, ratatui::style::Color)],
    total: u64,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let width = width.max(10);
    let total = total.max(1);
    let mut cells = Vec::new();
    let mut used = 0usize;
    for (index, (value, color)) in parts.iter().enumerate() {
        let cells_for_part = if index + 1 == parts.len() {
            width.saturating_sub(used)
        } else {
            ((*value as f64 / total as f64) * width as f64).round() as usize
        };
        let cells_for_part = cells_for_part.min(width.saturating_sub(used));
        if cells_for_part > 0 {
            cells.push(Span::styled("█".repeat(cells_for_part), theme.fg(*color)));
            used += cells_for_part;
        }
    }
    if used < width {
        cells.push(Span::styled("░".repeat(width - used), theme.mute()));
    }
    Line::from(cells)
}

fn usage_row(
    label: &str,
    value: u64,
    total: u64,
    color: ratatui::style::Color,
    theme: &Theme,
) -> Line<'static> {
    Line::from(vec![
        Span::styled("◆ ", theme.fg(color)),
        Span::styled(format!("{label:<12}"), theme.fg(theme.text)),
        Span::styled(
            format!("{:>8} tokens  ", fmt_tokens(value)),
            theme.fg(theme.text_dim),
        ),
        Span::styled(format!("{:>3}%", percentage(value, total)), theme.mute()),
    ])
}

fn render_rule(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let inner = inset(area);
    let line = "─".repeat(inner.width as usize);
    frame.render_widget(Paragraph::new(line).style(theme.fg(theme.hairline)), inner);
}

fn render_transcript(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
    cache: &mut TranscriptCache,
) {
    let inner = inset(area);
    hits.tool_summary_rows =
        crate::conversation::render_transcript(frame, inner, model, theme, cache);
}

/// Whether reasoning blocks are expanded. Toggled by `/reasoning`.
static SHOW_REASONING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Flip reasoning visibility and return the new state.
pub fn toggle_reasoning() -> bool {
    !SHOW_REASONING.fetch_xor(true, std::sync::atomic::Ordering::Relaxed)
}

fn show_reasoning() -> bool {
    SHOW_REASONING.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TranscriptCacheKey {
    session_id: String,
    sequence: u64,
    live: String,
    width: usize,
    search: String,
    hide_tools: bool,
    message_roles: bool,
    show_reasoning: bool,
    user_label: String,
    theme: Theme,
}

#[derive(Clone, Debug)]
pub(crate) struct ToolSummaryMark {
    line: usize,
    width: usize,
    indices: Vec<usize>,
}

struct RenderedTranscript {
    lines: Vec<Line<'static>>,
    marks: Vec<ToolSummaryMark>,
    live_visible: bool,
}

/// Cached transcript layout. `tick` repaints only the trailing spinner.
#[derive(Default)]
pub struct TranscriptCache {
    key: Option<TranscriptCacheKey>,
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) marks: Vec<ToolSummaryMark>,
    pub(crate) live_visible: bool,
    rebuilds: u64,
}

impl TranscriptCache {
    #[cfg(test)]
    fn rebuilds(&self) -> u64 {
        self.rebuilds
    }

    pub(crate) fn ensure(
        &mut self,
        projection: &SessionProjection,
        session_id: &str,
        search: Option<&str>,
        hide_tools: bool,
        message_roles: bool,
        width: usize,
        user_label: &str,
        theme: &Theme,
    ) {
        let key = TranscriptCacheKey {
            session_id: session_id.to_owned(),
            sequence: projection.last_sequence,
            live: projection.live_assistant().unwrap_or("").to_owned(),
            width,
            search: search.unwrap_or("").to_owned(),
            hide_tools,
            message_roles,
            show_reasoning: show_reasoning(),
            user_label: user_label.to_owned(),
            theme: *theme,
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        let rendered = transcript_lines(
            projection,
            search,
            hide_tools,
            message_roles,
            width,
            user_label,
            theme,
        );
        self.lines = rendered.lines;
        self.marks = rendered.marks;
        self.live_visible = rendered.live_visible;
        self.key = Some(key);
        self.rebuilds += 1;
    }
}

pub(crate) fn tool_summary_hit_rows(
    marks: &[ToolSummaryMark],
    line_count: usize,
    rect: Rect,
    scroll: usize,
) -> Vec<(Rect, Vec<usize>)> {
    let height = rect.height as usize;
    if height == 0 || line_count == 0 {
        return Vec::new();
    }
    let start = window_start(line_count, height, scroll);
    let mut rows = Vec::new();
    for mark in marks {
        if mark.line < start {
            continue;
        }
        let visible_row = mark.line - start;
        if visible_row >= height {
            continue;
        }
        let offset = 2u16.min(rect.width);
        let width = u16::try_from(mark.width)
            .unwrap_or(u16::MAX)
            .min(rect.width.saturating_sub(offset));
        if width > 0 {
            rows.push((
                Rect::new(rect.x + offset, rect.y + visible_row as u16, width, 1),
                mark.indices.clone(),
            ));
        }
    }
    rows
}

pub(crate) fn welcome_lines(width: usize, height: usize, theme: &Theme) -> Vec<Line<'static>> {
    let compact = width < 40 || height < 7;
    let mut lines = vec![
        Line::from(Span::styled(ellipsize("Blora", width), theme.rose_bold())),
        Line::from(Span::styled(
            ellipsize(&i18n::tr("canvas.start", "输入消息开始，/ 查看命令"), width),
            theme.dim(),
        )),
    ];
    if !compact {
        lines.insert(
            1,
            Line::from(Span::styled(
                ellipsize(
                    &i18n::tr("canvas.tagline", "本地优先 · 编程 · 工作 · 智能体"),
                    width,
                ),
                theme.mute(),
            )),
        );
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            ellipsize(
                &i18n::tr("canvas.shortcuts", "Ctrl+N 新建会话 · ↑/↓ 输入历史"),
                width,
            ),
            theme.mute(),
        )));
        lines.insert(0, Line::default());
    }
    lines.truncate(height);
    lines
}

pub(crate) fn composer_placeholder(imagine: bool, running: bool, width: usize) -> String {
    let (key, fallback) = if running {
        ("composer.queued", "输入补充，Enter 排队发送")
    } else if imagine {
        ("composer.imagine", "描述画面，或输入 / 命令")
    } else if width < 30 {
        ("composer.compact", "输入消息 / 命令")
    } else {
        ("composer.message", "输入消息，或 / 查看命令")
    };
    ellipsize(&i18n::tr(key, fallback), width)
}

fn workspace_status(workspace: &Path, status: &str, width: usize) -> String {
    let label = if status.is_empty() {
        workspace
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| workspace.display().to_string())
    } else {
        status.to_owned()
    };
    ellipsize(&label, width)
}

/// Imagine sessions list saved files instead of a coding transcript.
fn imagine_lines(
    projection: &SessionProjection,
    width: usize,
    theme: &Theme,
) -> RenderedTranscript {
    let mut out = vec![Line::from(Span::styled(
        "已生成",
        theme.fg(theme.text).add_modifier(Modifier::BOLD),
    ))];
    let mut prompt = String::new();
    let mut any = false;
    for item in &projection.transcript {
        match item {
            TranscriptItem::User { text, .. } => {
                prompt = text.lines().next().unwrap_or("").trim().to_owned();
            }
            TranscriptItem::Tool { status, output, .. } => {
                let output = output.as_deref().unwrap_or("");
                let mut images = Vec::new();
                for line in output.lines() {
                    if let Some(path) = image_path_token(line) {
                        images.push((path.to_owned(), line.trim().to_owned()));
                    }
                }
                if !images.is_empty() {
                    any = true;
                    for (path, line) in images {
                        out.push(Line::default());
                        let caption = if prompt.is_empty() {
                            path
                        } else {
                            prompt.clone()
                        };
                        for wrapped in wrap_text(&caption, width.saturating_sub(2).max(8)) {
                            out.push(Line::from(Span::styled(wrapped, theme.fg(theme.text))));
                        }
                        for wrapped in wrap_text(&line, width.saturating_sub(2).max(8)) {
                            out.push(Line::from(Span::styled(wrapped, theme.mute())));
                        }
                    }
                } else if (status == "failed" || status == "error")
                    && !output.contains("missing string field path")
                {
                    any = true;
                    let message = if output.trim().is_empty() {
                        "生成失败".to_owned()
                    } else {
                        output.trim().to_owned()
                    };
                    out.push(Line::default());
                    for wrapped in wrap_text(&message, width.saturating_sub(2).max(8))
                        .into_iter()
                        .take(4)
                    {
                        out.push(Line::from(Span::styled(wrapped, theme.fg(theme.rust))));
                    }
                }
            }
            _ => {}
        }
    }
    if !any {
        out.push(Line::default());
        out.push(Line::from(Span::styled(
            "还没有图片。在下面描述画面。",
            theme.mute(),
        )));
    }
    RenderedTranscript {
        lines: out,
        marks: Vec::new(),
        live_visible: false,
    }
}

fn image_path_token(line: &str) -> Option<&str> {
    let start = line.find(".blora/images/")?;
    let rest = &line[start..];
    let end = rest
        .find(|ch: char| ch.is_whitespace())
        .unwrap_or(rest.len());
    Some(rest[..end].trim_end_matches(|ch: char| matches!(ch, ',' | ')' | ']' | '.')))
}

fn transcript_lines(
    projection: &SessionProjection,
    search: Option<&str>,
    hide_tools: bool,
    message_roles: bool,
    width: usize,
    user_label: &str,
    theme: &Theme,
) -> RenderedTranscript {
    if projection
        .session
        .as_ref()
        .is_some_and(|session| session.mode == Mode::Imagine)
    {
        return imagine_lines(projection, width, theme);
    }
    let needle = search.map(str::to_ascii_lowercase);
    let show_reasoning = show_reasoning();
    let mut out = Vec::new();
    let mut marks = Vec::new();
    let visible: Vec<(usize, &TranscriptItem)> = projection
        .transcript
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            !(hide_tools && matches!(item, TranscriptItem::Tool { .. }))
                && item_matches(item, needle.as_deref())
        })
        .collect();
    let mut index = 0usize;
    while index < visible.len() {
        if !out.is_empty() {
            out.push(Line::default());
        }
        if matches!(visible[index].1, TranscriptItem::Tool { .. }) {
            let start = index;
            index += 1;
            while index < visible.len() && matches!(visible[index].1, TranscriptItem::Tool { .. }) {
                index += 1;
            }
            let group = &visible[start..index];
            let items = group.iter().map(|(_, item)| *item).collect::<Vec<_>>();
            let summary = blora_session::summarize_tool_run(&items, false);
            let failed = group.iter().any(|(_, item)| {
                matches!(item, TranscriptItem::Tool { status, .. } if status == "failed" || status == "error")
            });
            let color = if failed {
                theme.rust
            } else if group.iter().any(|(_, item)| {
                matches!(item, TranscriptItem::Tool { status, .. } if status == "running" || status == "requested")
            }) {
                theme.amber
            } else {
                theme.text_dim
            };
            marks.push(ToolSummaryMark {
                line: out.len(),
                width: summary.width(),
                indices: group.iter().map(|(item_index, _)| *item_index).collect(),
            });
            out.push(Line::from(vec![
                Span::styled("  ", theme.mute()),
                Span::styled(summary, theme.fg(color)),
            ]));
            continue;
        }
        let item = visible[index].1;
        index += 1;
        match item {
            TranscriptItem::User { text, .. } => {
                let label = if user_label.trim().is_empty() {
                    "you"
                } else {
                    user_label
                };
                push_block(
                    &mut out,
                    label,
                    theme.rose,
                    text,
                    width,
                    needle.as_deref(),
                    message_roles,
                    theme,
                );
            }
            TranscriptItem::Assistant { text, .. } => {
                push_block(
                    &mut out,
                    "Blora",
                    theme.sage,
                    text,
                    width,
                    needle.as_deref(),
                    message_roles,
                    theme,
                );
            }
            TranscriptItem::Tool {
                name,
                status,
                arguments,
                output,
                ..
            } => {
                let color = if status == "failed" || status == "error" {
                    theme.rust
                } else if status == "running" || status == "requested" {
                    theme.amber
                } else {
                    theme.sage
                };
                out.push(Line::from(vec![
                    Span::styled("┃ ", theme.fg(color)),
                    Span::styled(name.clone(), theme.fg(color).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  {status}"), theme.mute()),
                ]));
                let body_width = width.saturating_sub(2).max(8);
                if let Some(arguments) = arguments {
                    for line in wrap_text(arguments, body_width).into_iter().take(3) {
                        out.push(Line::from(vec![
                            Span::styled("  ", theme.mute()),
                            Span::styled(line, theme.mute()),
                        ]));
                    }
                }
                if let Some(output) = output {
                    for line in wrap_text(output, body_width).into_iter().take(12) {
                        let mut spans = vec![Span::styled("  ", theme.fg(theme.text))];
                        spans.extend(highlight_spans(&line, needle.as_deref(), theme));
                        out.push(Line::from(spans));
                    }
                }
            }
            TranscriptItem::System { summary, .. } => {
                out.push(Line::from(vec![
                    Span::styled("  ", theme.mute()),
                    Span::styled(summary.clone(), theme.mute().add_modifier(Modifier::ITALIC)),
                ]));
            }
            TranscriptItem::Reasoning { text, .. } => {
                let body_width = width.saturating_sub(4).max(8);
                if show_reasoning {
                    out.push(Line::from(vec![Span::styled(
                        format!("  {}", crate::i18n::tr("reasoning.label", "思考过程")),
                        theme.mute().add_modifier(Modifier::ITALIC),
                    )]));
                    for line in wrap_text(text, body_width) {
                        out.push(Line::from(vec![
                            Span::styled("  │ ", theme.mute()),
                            Span::styled(line, theme.mute().add_modifier(Modifier::ITALIC)),
                        ]));
                    }
                } else {
                    let words = text.chars().filter(|ch| !ch.is_whitespace()).count();
                    out.push(Line::from(vec![Span::styled(
                        format!(
                            "  ▸ {} · {words} {}",
                            crate::i18n::tr("reasoning.label", "思考过程"),
                            crate::i18n::tr("reasoning.chars", "字"),
                        ),
                        theme.mute().add_modifier(Modifier::ITALIC),
                    )]));
                }
            }
            TranscriptItem::Question {
                question,
                options,
                answer,
                status,
                ..
            } => {
                let accent = if status == "pending" {
                    theme.amber
                } else {
                    theme.text_dim
                };
                out.push(Line::from(vec![
                    Span::styled("┃ ", theme.fg(accent)),
                    Span::styled(
                        crate::i18n::tr("question.label", "Blora 提问"),
                        theme.fg(accent).add_modifier(Modifier::BOLD),
                    ),
                ]));
                let body_width = width.saturating_sub(2).max(8);
                for line in wrap_text(question, body_width) {
                    out.push(Line::from(vec![
                        Span::styled("  ", theme.mute()),
                        Span::styled(line, theme.fg(theme.text)),
                    ]));
                }
                if status == "pending" {
                    for (index, option) in options.iter().enumerate() {
                        out.push(Line::from(vec![Span::styled(
                            format!("  {}. {option}", index + 1),
                            theme.fg(theme.text_dim),
                        )]));
                    }
                }
                match (status.as_str(), answer) {
                    (_, Some(answer)) => out.push(Line::from(vec![
                        Span::styled("  → ", theme.fg(theme.sage)),
                        Span::styled(answer.clone(), theme.fg(theme.sage)),
                    ])),
                    ("dismissed", None) => out.push(Line::from(vec![Span::styled(
                        format!("  {}", crate::i18n::tr("question.dismissed", "已跳过")),
                        theme.mute(),
                    )])),
                    _ => {}
                }
            }
            TranscriptItem::Routing {
                from_provider,
                to_provider,
                from_model,
                to_model,
                at,
                ..
            } => {
                let when = blora_session::relative_zh(*at, chrono::Utc::now());
                let from_p = from_provider.as_deref().map(pretty_route_id);
                let to_p = pretty_route_id(to_provider);
                let body = blora_session::format_routing_switch(
                    from_p.as_deref(),
                    &to_p,
                    from_model.as_deref(),
                    to_model,
                );
                out.push(Line::from(vec![
                    Span::styled("┃ ", theme.fg(theme.amber)),
                    Span::styled(when, theme.fg(theme.amber)),
                ]));
                let body_width = width.saturating_sub(2).max(8);
                for line in wrap_text(&body, body_width) {
                    out.push(Line::from(vec![
                        Span::styled("  ", theme.mute()),
                        Span::styled(line, theme.fg(theme.text_dim)),
                    ]));
                }
            }
        }
    }
    let mut live_visible = false;
    if let Some(live) = projection.live_assistant() {
        let show = match needle.as_deref() {
            None => true,
            Some(needle) => live.to_ascii_lowercase().contains(needle),
        };
        if show {
            live_visible = true;
            if !out.is_empty() {
                out.push(Line::default());
            }
            push_block(
                &mut out,
                "blora",
                theme.sage,
                live,
                width,
                needle.as_deref(),
                message_roles,
                theme,
            );
        }
    }
    RenderedTranscript {
        lines: out,
        marks,
        live_visible,
    }
}

pub(crate) fn spinner_line(tick: u64, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled("  ", theme.mute()),
        Span::styled(spinner(tick).to_string(), theme.fg(theme.sage)),
    ])
}

/// Keep the latest lines in view. `scroll` is how many lines above the bottom
/// to reveal; 0 always shows the newest message.
pub(crate) fn highlight_selection(
    lines: Vec<Line<'static>>,
    rect: Rect,
    selection: Option<Selection>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let Some(selection) = selection else {
        return lines;
    };
    let start_row = selection.start.1.min(selection.end.1);
    let end_row = selection.start.1.max(selection.end.1);
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let row = rect.y.saturating_add(index as u16);
            if row < start_row || row > end_row {
                return line;
            }
            let start_col = if row == start_row {
                if selection.start.1 <= selection.end.1 {
                    selection.start.0
                } else {
                    selection.end.0
                }
            } else {
                rect.x
            };
            let end_col = if row == end_row {
                if selection.start.1 <= selection.end.1 {
                    selection.end.0
                } else {
                    selection.start.0
                }
            } else {
                rect.right()
            };
            let from = start_col.saturating_sub(rect.x) as usize;
            let to = end_col.saturating_sub(rect.x) as usize;
            let text = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            let chars: Vec<char> = text.chars().collect();
            let from = from.min(chars.len());
            let to = to.min(chars.len()).max(from);
            let mut spans = Vec::new();
            if from > 0 {
                spans.push(Span::styled(
                    chars[..from].iter().collect::<String>(),
                    theme.base(),
                ));
            }
            if to > from {
                spans.push(Span::styled(
                    chars[from..to].iter().collect::<String>(),
                    theme
                        .fg(theme.bg)
                        .bg(theme.sage)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            if to < chars.len() {
                spans.push(Span::styled(
                    chars[to..].iter().collect::<String>(),
                    theme.base(),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

fn item_matches(item: &TranscriptItem, needle: Option<&str>) -> bool {
    let Some(needle) = needle else {
        return true;
    };
    match item {
        TranscriptItem::User { text, .. } | TranscriptItem::Assistant { text, .. } => {
            text.to_ascii_lowercase().contains(needle)
        }
        TranscriptItem::Tool {
            name,
            arguments,
            output,
            ..
        } => {
            name.to_ascii_lowercase().contains(needle)
                || arguments
                    .as_deref()
                    .is_some_and(|text| text.to_ascii_lowercase().contains(needle))
                || output
                    .as_deref()
                    .is_some_and(|text| text.to_ascii_lowercase().contains(needle))
        }
        TranscriptItem::System { summary, .. } => summary.to_ascii_lowercase().contains(needle),
        TranscriptItem::Reasoning { text, .. } => text.to_ascii_lowercase().contains(needle),
        TranscriptItem::Question {
            question, answer, ..
        } => {
            question.to_ascii_lowercase().contains(needle)
                || answer
                    .as_deref()
                    .is_some_and(|text| text.to_ascii_lowercase().contains(needle))
        }
        TranscriptItem::Routing {
            to_provider,
            to_model,
            from_provider,
            from_model,
            ..
        } => {
            to_provider.to_ascii_lowercase().contains(needle)
                || to_model.to_ascii_lowercase().contains(needle)
                || from_provider
                    .as_deref()
                    .is_some_and(|value| value.to_ascii_lowercase().contains(needle))
                || from_model
                    .as_deref()
                    .is_some_and(|value| value.to_ascii_lowercase().contains(needle))
        }
    }
}

fn window_start(total: usize, height: usize, scroll: usize) -> usize {
    if height == 0 || total <= height {
        return 0;
    }
    let max_scroll = total - height;
    let from_bottom = scroll.min(max_scroll);
    total - height - from_bottom
}

pub(crate) fn transcript_window(
    lines: &[Line<'static>],
    spinner: Option<Line<'static>>,
    height: usize,
    scroll: usize,
) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    let total = lines.len() + usize::from(spinner.is_some());
    if total == 0 {
        return Vec::new();
    }
    let start = window_start(total, height, scroll);
    let mut visible = Vec::new();
    if start < lines.len() {
        let end = (start + height).min(lines.len());
        visible.extend(lines[start..end].iter().cloned());
    }
    if let Some(spinner) = spinner
        && visible.len() < height
        && start + visible.len() < total
    {
        visible.push(spinner);
    }
    visible
}

fn push_block(
    out: &mut Vec<Line<'static>>,
    role: &str,
    accent: ratatui::style::Color,
    text: &str,
    width: usize,
    needle: Option<&str>,
    message_roles: bool,
    theme: &Theme,
) {
    if message_roles {
        out.push(Line::from(vec![
            Span::styled("┃ ", theme.fg(accent)),
            Span::styled(
                role.to_owned(),
                theme.fg(accent).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    let body_width = width.saturating_sub(2).max(8);
    out.extend(crate::markdown::render(text, body_width, needle, theme));
}

fn highlight_spans(text: &str, needle: Option<&str>, theme: &Theme) -> Vec<Span<'static>> {
    let Some(needle) = needle.filter(|item| !item.is_empty()) else {
        return vec![Span::styled(text.to_owned(), theme.fg(theme.text))];
    };
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut rest = text;
    let mut rest_lower = lower.as_str();
    while let Some(at) = rest_lower.find(needle) {
        if at > 0 {
            spans.push(Span::styled(rest[..at].to_owned(), theme.fg(theme.text)));
        }
        let end = at + needle.len();
        spans.push(Span::styled(
            rest[at..end].to_owned(),
            theme.fg(theme.amber).add_modifier(Modifier::BOLD),
        ));
        rest = &rest[end..];
        rest_lower = &rest_lower[end..];
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_owned(), theme.fg(theme.text)));
    }
    if spans.is_empty() {
        spans.push(Span::styled(text.to_owned(), theme.fg(theme.text)));
    }
    spans
}

fn render_approval(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let inner = inset(area);
    let Some(first) = model.pending.first() else {
        return;
    };
    let owner = model
        .right_panel
        .and_then(|panel| panel.chat.as_ref())
        .filter(|chat| chat.id == first.session_id)
        .map(|chat| chat.title());
    let summary_text = owner
        .map(|owner| format!("[{owner}] {}", first.summary))
        .unwrap_or_else(|| first.summary.clone());
    let summary = ellipsize(&summary_text, inner.width.saturating_sub(22) as usize);
    let allow_text = "y allow";
    let deny_text = "n deny";
    let mut x =
        inner.x + u16::try_from("!  ".width() + summary.width() + "   ".width()).unwrap_or(0);
    hits.allow = Some(Rect {
        x,
        y: inner.y,
        width: u16::try_from(allow_text.width()).unwrap_or(7),
        height: 1,
    });
    x = x.saturating_add(u16::try_from(allow_text.width() + "   ".width()).unwrap_or(10));
    hits.deny = Some(Rect {
        x,
        y: inner.y,
        width: u16::try_from(deny_text.width()).unwrap_or(6),
        height: 1,
    });
    let line = Line::from(vec![
        Span::styled("!  ", theme.fg(theme.amber).add_modifier(Modifier::BOLD)),
        Span::styled(summary, theme.fg(theme.amber)),
        Span::styled("   ", theme.mute()),
        Span::styled(allow_text, theme.sage_bold()),
        Span::styled("   ", theme.mute()),
        Span::styled(deny_text, theme.fg(theme.rust).add_modifier(Modifier::BOLD)),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.bg_raised)),
        inner,
    );
}

fn render_slash(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let inner = inset(area);
    let capacity = usize::from(inner.height).min(slash::MAX_VISIBLE);
    if capacity == 0 {
        return;
    }
    let start = model.slash_selected.saturating_sub(capacity - 1);
    let end = model.slash_hits.len().min(start + capacity);
    let visible = if model.slash_hits.is_empty() {
        &[][..]
    } else {
        &model.slash_hits[start..end]
    };
    let label_w = visible
        .iter()
        .map(|cmd| slash_label(cmd).width())
        .max()
        .unwrap_or(8)
        .min((inner.width as usize).saturating_mul(3) / 5)
        .max(6);
    let hover_row = model.pointer.and_then(|(col, row)| {
        if contains(inner, col, row) {
            Some(usize::from(row.saturating_sub(inner.y)))
        } else {
            None
        }
    });
    let items: Vec<ListItem> = if visible.is_empty() {
        vec![ListItem::new(Span::styled(
            "  no matching command",
            theme.mute(),
        ))]
    } else {
        visible
            .iter()
            .enumerate()
            .map(|(idx, cmd)| {
                let abs = start + idx;
                let selected = abs == model.slash_selected;
                let hovered = hover_row == Some(idx);
                let prefix = if selected { "❯ " } else { "  " };
                let label = pad_right(&slash_label(cmd), label_w);
                hits.slash_rows.push((
                    Rect {
                        x: inner.x,
                        y: inner.y.saturating_add(u16::try_from(idx).unwrap_or(0)),
                        width: inner.width,
                        height: 1,
                    },
                    abs,
                ));
                let bg = if selected || hovered {
                    theme.bg_select
                } else {
                    theme.bg_raised
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{prefix}{label}"),
                        theme.fg(theme.sage).add_modifier(Modifier::BOLD).bg(bg),
                    ),
                    Span::styled("  ", Style::default().bg(bg)),
                    Span::styled(cmd.about, theme.mute().bg(bg)),
                ]))
            })
            .collect()
    };
    let mut state = ListState::default();
    if !visible.is_empty() {
        state.select(Some(model.slash_selected.saturating_sub(start)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .style(Style::default().bg(theme.bg_raised).fg(theme.text))
            .highlight_style(
                Style::default()
                    .bg(theme.bg_select)
                    .fg(theme.text)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(""),
        inner,
        &mut state,
    );
}

fn slash_label(cmd: &SlashCommand) -> String {
    if cmd.hint.is_empty() {
        format!("/{}", cmd.name)
    } else {
        format!("/{} {}", cmd.name, cmd.hint)
    }
}

fn render_notice(frame: &mut Frame<'_>, area: Rect, body: &str, theme: &Theme) {
    let inner = inset(area);
    let lines: Vec<Line> = body
        .lines()
        .map(|line| {
            Line::from(vec![
                Span::styled("│ ", theme.fg(theme.sage)),
                Span::styled(line.to_owned(), theme.dim()),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.bg_raised)),
        inner,
    );
}

/// PassPort login dialog, centered like the modal dialogs of the reference
/// harness: rounded frame on a cleared background, mac-style traffic lights,
/// dim body rows, and a bottom key hint (`esc = …`). Pure overlay: closing it
/// only hides it. The traffic lights are clickable like their macOS
/// counterparts: red quits, yellow hides, green reopens the browser.
fn render_passport_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &PassportDialog,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap::default();
    const DIALOG_HEIGHT: u16 = 8;
    const MIN_WIDTH: u16 = 46;
    if area.width < 20 || area.height < DIALOG_HEIGHT {
        return hits;
    }
    let uri_width = dialog.verification_uri.width() as u16;
    let dialog_width = (uri_width + 8).clamp(MIN_WIDTH, area.width.saturating_sub(4));
    let Some(frame_y) = dialog_outer(area, dialog_width, DIALOG_HEIGHT, false, false) else {
        return hits;
    };
    let inner = paint_dialog_frame(
        frame,
        frame_y,
        &i18n::tr("dialog.passport_login", "Bloret PassPort 登录"),
        pointer,
        theme,
        &mut hits,
    );

    let rows: [Rect; 5] = split_dialog_rows(inner, DIALOG_HEIGHT - 2);
    let wrap_width = inner.width.saturating_sub(2) as usize;

    // Row 1: instructions.
    let instructions = wrap_text(
        &i18n::tr(
            "passport.instructions",
            "在浏览器打开下面的链接并输入设备码，授权后这里会自动登录。",
        ),
        wrap_width,
    );
    let mut instruction_lines: Vec<Line> = instructions
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.mute())))
        .collect();
    instruction_lines.resize(rows[1].height as usize, Line::default());
    frame.render_widget(
        Paragraph::new(instruction_lines).style(theme.base()),
        rows[1],
    );

    // Row 2: the device code, dimmed like the rest of the body.
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            ellipsize(&dialog.user_code, rows[2].width.saturating_sub(2) as usize),
            theme.dim(),
        )))
        .style(theme.base()),
        rows[2],
    );

    // Row 3: the verification link, dimmed like the rest of the body.
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            ellipsize(&dialog.verification_uri, rows[3].width as usize),
            theme.mute(),
        )))
        .style(theme.base()),
        rows[3],
    );

    // Row 4: status + key hint, everything muted.
    let waiting = if dialog.opened_browser {
        i18n::tr("passport.browser_opened", "已打开浏览器")
    } else {
        i18n::tr("passport.waiting", "等待授权")
    };
    let hint = Line::from(vec![
        Span::styled("esc", theme.mute()),
        Span::styled(
            i18n::tr("passport.hide_hint", " 隐藏对话框 · "),
            theme.mute(),
        ),
        Span::styled(waiting.to_owned(), theme.mute()),
    ]);
    frame.render_widget(Paragraph::new(hint).style(theme.base()), rows[4]);
    hits
}

/// Two-pane provider/model switch dialog.
fn render_provider_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &ProviderDialog,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap::default();
    let models = dialog.current_models();
    let row_count = dialog.options.len().max(models.len()).clamp(1, 8) as u16;
    let Some(frame_area) = dialog_outer(
        area,
        96,
        (row_count * 2 + 6).max(12),
        dialog.fullscreen,
        dialog.minimized,
    ) else {
        return hits;
    };
    let inner = paint_dialog_frame(
        frame,
        frame_area,
        &format!(
            "{} · {}",
            i18n::tr("dialog.provider_model", "选择供应商与模型"),
            i18n::tr("harness.native_settings", "Blora 原生设置")
        ),
        pointer,
        theme,
        &mut hits,
    );
    if dialog.minimized || inner.height <= 1 {
        return hits;
    }
    let inner = Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1);
    let (left_title, right_title, left, divider, right, hint_row) = if inner.width < 60 {
        let [left_title, left, right_title, right, hint_row] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        (
            left_title,
            right_title,
            left,
            Rect::default(),
            right,
            hint_row,
        )
    } else {
        let [headings, body, hint_row] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(inner);
        let columns = [
            Constraint::Percentage(42),
            Constraint::Length(1),
            Constraint::Min(0),
        ];
        let [left_title, _, right_title] = Layout::horizontal(columns).areas(headings);
        let [left, divider, right] = Layout::horizontal(columns).areas(body);
        (left_title, right_title, left, divider, right, hint_row)
    };
    for (rect, label, active) in [
        (left_title, "供应商", dialog.pane == ProviderPane::Providers),
        (right_title, "模型", dialog.pane == ProviderPane::Models),
    ] {
        frame.render_widget(
            Paragraph::new(format!(" {label}")).style(if active {
                theme.rose_bold()
            } else {
                theme.dim()
            }),
            rect,
        );
    }
    frame.render_widget(
        Paragraph::new(vec![Line::from("│"); divider.height as usize]).style(theme.mute()),
        divider,
    );
    // Keep the keyboard selection visible in lists taller than the dialog.
    let capacity = usize::from(left.height / 2).max(1);
    let provider_start = dialog.selected.saturating_sub(capacity - 1);
    for (index, option) in dialog
        .options
        .iter()
        .enumerate()
        .skip(provider_start)
        .take(capacity)
    {
        let y = left.y + ((index - provider_start) * 2) as u16;
        if y >= left.bottom() {
            break;
        }
        let rect = Rect::new(left.x, y, left.width, 2.min(left.bottom() - y));
        hits.provider_rows.push((rect, index));
        let hovered = pointer.is_some_and(|(col, row)| contains(rect, col, row));
        let selected = index == dialog.selected;
        let marker = if hovered || (selected && dialog.pane == ProviderPane::Providers) {
            "❯"
        } else if selected {
            "•"
        } else {
            " "
        };
        let hint = if option.available {
            option.hint.clone()
        } else {
            format!("不可用 · {}", option.hint)
        };
        let width = usize::from(rect.width.saturating_sub(3));
        let lines = vec![
            Line::from(vec![
                Span::styled(format!(" {marker} "), theme.fg(theme.rose)),
                Span::styled(
                    ellipsize(&option.display, width),
                    if selected {
                        theme.fg(theme.text)
                    } else {
                        theme.dim()
                    },
                ),
            ]),
            Line::from(Span::styled(
                format!("   {}", ellipsize(&hint, width)),
                theme.mute(),
            )),
        ];
        frame.render_widget(
            Paragraph::new(lines).style(if selected || hovered {
                theme.base().bg(theme.bg_select)
            } else {
                theme.base()
            }),
            rect,
        );
    }
    if dialog.is_add() || models.is_empty() {
        let text = if dialog.is_add() {
            "回车打开添加供应商向导".to_owned()
        } else {
            i18n::tr("provider.no_models", "没有可列出的模型")
        };
        frame.render_widget(
            Paragraph::new(ellipsize(&text, right.width as usize)).style(theme.mute()),
            right,
        );
    } else {
        let capacity = usize::from(right.height / 2).max(1);
        let start = dialog.model_selected.saturating_sub(capacity - 1);
        for (index, option) in models.iter().enumerate().skip(start).take(capacity) {
            let y = right.y + ((index - start) * 2) as u16;
            if y >= right.bottom() {
                break;
            }
            let rect = Rect::new(right.x, y, right.width, 2.min(right.bottom() - y));
            hits.provider_model_rows.push((rect, index));
            let hovered = pointer.is_some_and(|(col, row)| contains(rect, col, row));
            let selected = index == dialog.model_selected;
            let marker = if hovered || (selected && dialog.pane == ProviderPane::Models) {
                "❯"
            } else if selected {
                "•"
            } else {
                " "
            };
            let width = usize::from(rect.width.saturating_sub(3));
            let lines = vec![
                Line::from(vec![
                    Span::styled(format!(" {marker} "), theme.fg(theme.rose)),
                    Span::styled(
                        ellipsize(&option.display, width),
                        if selected {
                            theme.fg(theme.text)
                        } else {
                            theme.dim()
                        },
                    ),
                ]),
                Line::from(Span::styled(
                    format!("   {}", ellipsize(&option.hint, width)),
                    theme.mute(),
                )),
            ];
            frame.render_widget(
                Paragraph::new(lines).style(if selected || hovered {
                    theme.base().bg(theme.bg_select)
                } else {
                    theme.base()
                }),
                rect,
            );
        }
    }
    let hint = if hint_row.width >= 58 {
        " ↑/↓ 选择 · ←/→ 切换栏 · Enter 确认 · Esc 关闭"
    } else {
        " ↑↓ 选择 · ←→ 切换 · Enter 确认 · Esc"
    };
    frame.render_widget(
        Paragraph::new(ellipsize(hint, hint_row.width as usize)).style(theme.mute()),
        hint_row,
    );
    hits
}

fn render_tool_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &ToolDialog,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap::default();
    let height = (dialog.tools.len() as u16 * 4 + 4).clamp(8, area.height.saturating_sub(2));
    let Some(frame_area) = dialog_outer(area, 88, height, dialog.fullscreen, dialog.minimized)
    else {
        return hits;
    };
    let inner = paint_dialog_frame(frame, frame_area, "详细工具调用", pointer, theme, &mut hits);
    let [_, body, hint_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(inner);

    if dialog.minimized {
        return hits;
    }
    let mut lines = Vec::new();
    let mut row = 0usize;
    for (index, tool) in dialog.tools.iter().enumerate() {
        let row_rect = Rect::new(body.x, body.y + row as u16, body.width, 1);
        if row >= dialog.scroll && row - dialog.scroll < body.height as usize {
            hits.tool_detail_rows.push((
                Rect::new(
                    row_rect.x,
                    row_rect.y - dialog.scroll as u16,
                    row_rect.width,
                    1,
                ),
                index,
            ));
        }
        lines.push(Line::from(vec![
            Span::styled(format!("{}  ", index + 1), theme.fg(theme.rose)),
            Span::styled(
                tool_label(&tool.name),
                theme.fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {}", status_label(&tool.status)), theme.mute()),
        ]));
        row += 1;
        lines.push(Line::default());
        row += 1;
    }
    let visible = lines
        .into_iter()
        .skip(dialog.scroll)
        .take(body.height as usize)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(visible).style(theme.base()), body);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "↑/↓ 滚动 · Esc 关闭",
            theme.mute(),
        )))
        .style(theme.base()),
        hint_row,
    );
    hits
}

fn tool_label(name: &str) -> String {
    match name {
        "update_plan" => "更新任务计划".to_owned(),
        "read_file" => "读取文件".to_owned(),
        "write_file" => "写入文件".to_owned(),
        "apply_patch" | "edit" => "编辑文件".to_owned(),
        "search" | "grep" => "搜索内容".to_owned(),
        "glob" => "查找文件".to_owned(),
        "list_dir" => "列出目录".to_owned(),
        "git_status" => "查看 Git 状态".to_owned(),
        "git_diff" => "查看 Git 差异".to_owned(),
        "git_log" => "查看 Git 提交记录".to_owned(),
        "shell" | "bash" | "exec" => "执行命令".to_owned(),
        "todo_write" | "todo" => "更新任务清单".to_owned(),
        "delegate" => "派发子代理".to_owned(),
        _ => name.to_owned(),
    }
}

fn status_label(status: &str) -> &str {
    match status {
        "completed" => "已完成",
        "running" => "执行中",
        "requested" => "等待执行",
        "failed" | "error" => "失败",
        _ => status,
    }
}

fn tool_detail_lines(tool: &ToolCallDetail, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let content_width = width.saturating_sub(4).max(12);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("工具：", theme.fg(theme.text_dim)),
            Span::styled(
                tool_label(&tool.name),
                theme.fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  ({})", tool.name), theme.mute()),
        ]),
        Line::from(vec![
            Span::styled("状态：", theme.fg(theme.text_dim)),
            Span::styled(status_label(&tool.status).to_owned(), theme.fg(theme.sage)),
        ]),
        Line::default(),
    ];
    if let Some(arguments) = &tool.arguments {
        lines.push(Line::from(Span::styled(
            "调用参数",
            theme.fg(theme.rose).add_modifier(Modifier::BOLD),
        )));
        lines.extend(render_arguments(arguments, content_width, theme));
        lines.push(Line::default());
    }
    if let Some(output) = &tool.output {
        lines.push(Line::from(Span::styled(
            "工具输出",
            theme.fg(theme.rose).add_modifier(Modifier::BOLD),
        )));
        lines.extend(wrap_labeled_text(
            "  ",
            &output.clone(),
            content_width,
            theme.fg(theme.text_dim),
        ));
    }
    lines
}

fn render_arguments(arguments: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return wrap_labeled_text("  ", arguments, width, theme.mute());
    };
    let Some(object) = value.as_object() else {
        return wrap_labeled_text("  ", &pretty_json(&value), width, theme.mute());
    };
    let mut lines = Vec::new();
    for (key, value) in object {
        let label = argument_label(key);
        if value.is_object() || value.is_array() {
            lines.push(Line::from(vec![Span::styled(
                format!("  {label}："),
                theme.fg(theme.text_dim),
            )]));
            for line in pretty_json(value).lines() {
                lines.push(Line::from(Span::styled(
                    format!("    {line}"),
                    theme.mute(),
                )));
            }
        } else {
            lines.extend(wrap_labeled_text(
                &format!("  {label}："),
                &pretty_json(value),
                width,
                theme.mute(),
            ));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("  （无参数）", theme.mute())));
    }
    lines
}

fn argument_label(key: &str) -> String {
    match key {
        "path" => "路径".to_owned(),
        "pattern" => "匹配模式".to_owned(),
        "include" => "包含文件".to_owned(),
        "max_results" => "最大结果数".to_owned(),
        "offset" => "起始位置".to_owned(),
        "limit" => "读取行数".to_owned(),
        "query" => "搜索内容".to_owned(),
        "command" => "命令".to_owned(),
        "note" => "备注".to_owned(),
        "steps" => "步骤".to_owned(),
        "status" => "状态".to_owned(),
        "title" => "标题".to_owned(),
        _ => key.to_owned(),
    }
}

fn pretty_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

fn wrap_labeled_text(prefix: &str, text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for source_line in text.lines() {
        let chunks = wrap_text(source_line, width.saturating_sub(prefix.width()).max(8));
        if chunks.is_empty() {
            lines.push(Line::from(Span::styled(prefix.to_owned(), style)));
        } else {
            for chunk in chunks {
                lines.push(Line::from(Span::styled(format!("{prefix}{chunk}"), style)));
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(prefix.to_owned(), style)));
    }
    lines
}

fn render_git_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &GitDialog,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
    tick: u64,
) -> HitMap {
    let mut hits = HitMap::default();
    let Some(frame_area) = dialog_outer(area, 132, 28, dialog.fullscreen, dialog.minimized) else {
        return hits;
    };
    let inner = paint_dialog_frame(frame, frame_area, "Git", pointer, theme, &mut hits);
    let [_, tabs, body, hint_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(inner);
    if dialog.minimized {
        return hits;
    }
    let labels = ["操作", "Git 图", "输出"];
    for (index, label) in labels.iter().enumerate() {
        let start = tabs.x + tabs.width * index as u16 / labels.len() as u16;
        let end = tabs.x + tabs.width * (index as u16 + 1) / labels.len() as u16;
        let rect = Rect::new(start, tabs.y, end - start, 1);
        hits.git_tabs.push((rect, index));
        let selected = dialog.page == index;
        let style = if selected {
            theme
                .fg(theme.text)
                .bg(theme.bg_select)
                .add_modifier(Modifier::BOLD)
        } else {
            theme.mute()
        };
        frame.render_widget(
            Paragraph::new(format!(" {} {} ", if selected { "▸" } else { " " }, label))
                .style(style),
            rect,
        );
    }
    match dialog.page {
        0 => render_git_commit_page(frame, body, dialog, theme, &mut hits, tick),
        1 => {
            let lines = git_graph_lines(&dialog.info.log, theme);
            frame.render_widget(
                Paragraph::new(
                    lines
                        .into_iter()
                        .skip(
                            dialog
                                .scroll
                                .min(dialog.info.log.lines().count().saturating_sub(1)),
                        )
                        .take(body.height as usize)
                        .collect::<Vec<_>>(),
                )
                .style(theme.base()),
                body,
            );
        }
        2 => {
            let lines = git_feedback_lines(dialog.feedback.as_deref(), body.width as usize);
            let scroll = dialog
                .scroll
                .min(lines.len().saturating_sub(body.height as usize));
            frame.render_widget(
                Paragraph::new(
                    lines
                        .into_iter()
                        .skip(scroll)
                        .map(Line::from)
                        .collect::<Vec<_>>(),
                )
                .style(theme.fg(theme.amber)),
                body,
            );
        }
        _ => render_git_commit_page(frame, body, dialog, theme, &mut hits, tick),
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            if dialog.page == 0 {
                "单击选中 · 双击/空格勾选 · PgUp/Dn 差异 · Enter 提交 · Esc 关闭"
            } else if dialog.page == 1 {
                "↑/↓ 或滚轮浏览 · ←/→ 分页 · 最近 100 次提交 · Esc 关闭"
            } else {
                "↑/↓、PgUp/Dn 或滚轮浏览完整输出 · ←/→ 分页 · Esc 关闭"
            },
            theme.mute(),
        )))
        .style(theme.base()),
        hint_row,
    );
    hits
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitChangedFile {
    pub original_path: Option<String>,
    pub path: String,
    pub status: String,
    pub staged: bool,
}

fn git_feedback_lines(feedback: Option<&str>, width: usize) -> Vec<String> {
    wrap_text(feedback.unwrap_or("暂无操作输出"), width)
}

fn decode_git_path(path: &str) -> String {
    let Some(quoted) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return path.to_owned();
    };
    let mut bytes = quoted.bytes().peekable();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let Some(escaped) = bytes.next() else { break };
        if (b'0'..=b'7').contains(&escaped) {
            let mut value = escaped - b'0';
            for _ in 0..2 {
                if let Some(next) = bytes.next_if(|next| (b'0'..=b'7').contains(next)) {
                    value = value.wrapping_mul(8).wrapping_add(next - b'0');
                } else {
                    break;
                }
            }
            decoded.push(value);
        } else {
            decoded.push(match escaped {
                b'a' => 7,
                b'b' => 8,
                b't' => b'\t',
                b'n' => b'\n',
                b'v' => 11,
                b'f' => 12,
                b'r' => b'\r',
                other => other,
            });
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

pub fn git_changed_files(raw: &str) -> Vec<GitChangedFile> {
    raw.lines()
        .skip(1)
        .filter_map(|line| {
            let status = line.get(..2)?.to_owned();
            let path = line.get(3..)?.to_owned();
            if path.is_empty() || status == "!!" {
                return None;
            }
            let (original_path, path) = if status.contains(['R', 'C']) {
                let mut quoted = false;
                let mut escaped = false;
                let split = path.char_indices().find_map(|(index, ch)| {
                    if escaped {
                        escaped = false;
                    } else if quoted && ch == '\\' {
                        escaped = true;
                    } else if ch == '"' {
                        quoted = !quoted;
                    } else if !quoted && path[index..].starts_with(" -> ") {
                        return Some(index);
                    }
                    None
                })?;
                let (old, new) = (&path[..split], &path[split + 4..]);
                (
                    status.contains('R').then(|| decode_git_path(old)),
                    decode_git_path(new),
                )
            } else {
                (None, decode_git_path(&path))
            };
            Some(GitChangedFile {
                original_path,
                staged: status.as_bytes()[0] != b' ' && status.as_bytes()[0] != b'?',
                path,
                status,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitPrimaryAction {
    Commit(usize),
    Pull,
    Push,
    None,
}

pub fn git_primary_action(
    info: &GitStatusInfo,
    excluded: &std::collections::HashSet<String>,
) -> GitPrimaryAction {
    let files = git_changed_files(&info.raw);
    let count = files
        .iter()
        .filter(|file| !excluded.contains(&file.path))
        .count();
    if !files.is_empty() {
        GitPrimaryAction::Commit(count)
    } else if info.behind > 0 {
        GitPrimaryAction::Pull
    } else if info.ahead > 0 {
        GitPrimaryAction::Push
    } else {
        GitPrimaryAction::None
    }
}

fn git_primary_label(info: &GitStatusInfo, excluded: &std::collections::HashSet<String>) -> String {
    let counts = format!(
        "{}{}",
        if info.ahead > 0 {
            format!(" ↑{}", info.ahead)
        } else {
            String::new()
        },
        if info.behind > 0 {
            format!(" ↓{}", info.behind)
        } else {
            String::new()
        },
    );
    match git_primary_action(info, excluded) {
        GitPrimaryAction::Commit(count) => format!("提交 {count} 个更改"),
        GitPrimaryAction::Pull => format!("拉取{counts}"),
        GitPrimaryAction::Push => format!("推送{counts}"),
        GitPrimaryAction::None => "已同步".to_owned(),
    }
}

fn render_git_commit_page(
    frame: &mut Frame<'_>,
    body: Rect,
    dialog: &GitDialog,
    theme: &Theme,
    hits: &mut HitMap,
    tick: u64,
) {
    if body.height < 5 || body.width < 20 {
        return;
    }
    let (body, diff_area) = if dialog.diff_path.is_some() && body.width >= 72 {
        let [list, diff] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                .areas(body);
        (list, Some(diff))
    } else if dialog.diff_path.is_some() && body.height >= 14 {
        let [list, diff] =
            Layout::vertical([Constraint::Length(9), Constraint::Min(5)]).areas(body);
        (list, Some(diff))
    } else {
        (body, None)
    };
    if let Some(diff) = diff_area {
        render_git_diff(frame, diff, dialog, theme, hits);
    }
    let message = Rect::new(body.x + 1, body.y + 1, body.width.saturating_sub(2), 1);
    let left_width = message.width / 2;
    let generate = Rect::new(message.x, body.y + 2, left_width.saturating_sub(1), 1);
    let primary = Rect::new(
        message.x + left_width,
        body.y + 2,
        message.width - left_width,
        1,
    );
    hits.git_message = Some(message);
    hits.git_generate = Some(generate);
    hits.git_primary = Some(primary);
    let message_text = if dialog.message.is_empty() {
        format!(
            "提交消息（在 {} 上提交）",
            dialog
                .info
                .raw
                .lines()
                .next()
                .unwrap_or("当前分支")
                .trim_start_matches("## ")
        )
    } else {
        dialog.message.clone()
    };
    frame.render_widget(
        Paragraph::new(format!(
            " {}{}",
            message_text,
            if dialog.editing_message { "▏" } else { "" }
        ))
        .style(
            theme
                .fg(if dialog.message.is_empty() {
                    theme.text_dim
                } else {
                    theme.text
                })
                .bg(theme.bg_raised),
        ),
        message,
    );
    let generate_label = if dialog.generating_message {
        format!(" {} 正在生成提交信息…", spinner(tick))
    } else {
        " ✦ 生成提交信息".to_owned()
    };
    frame.render_widget(
        Paragraph::new(generate_label).style(theme.fg(theme.sage).bg(theme.bg_raised)),
        generate,
    );
    let primary_label = if dialog.syncing {
        format!(" {} 正在同步…", spinner(tick))
    } else {
        format!(
            " {}",
            git_primary_label(&dialog.info, &dialog.excluded_files)
        )
    };
    let primary_style = if matches!(
        git_primary_action(&dialog.info, &dialog.excluded_files),
        GitPrimaryAction::None | GitPrimaryAction::Commit(0)
    ) {
        theme.fg(theme.text_dim).bg(theme.bg_raised)
    } else {
        theme
            .fg(theme.text)
            .bg(theme.bg_select)
            .add_modifier(Modifier::BOLD)
    };
    frame.render_widget(Paragraph::new(primary_label).style(primary_style), primary);
    let files = git_changed_files(&dialog.info.raw);
    let selected = files
        .iter()
        .filter(|file| !dialog.excluded_files.contains(&file.path))
        .count();
    let summary = Rect::new(message.x, body.y + 3, message.width, 1);
    if summary.y >= body.bottom() {
        return;
    }
    frame.render_widget(
        Paragraph::new(format!(
            "更改 {} · 已勾选 {} · 双击/空格勾选",
            files.len(),
            selected
        ))
        .style(theme.fg(theme.text_dim).add_modifier(Modifier::BOLD)),
        summary,
    );
    if files.is_empty() {
        let row = Rect::new(message.x, summary.y + 1, message.width, 1);
        frame.render_widget(
            Paragraph::new("  工作区没有待提交的更改").style(theme.mute()),
            row,
        );
    }
    let visible = body
        .bottom()
        .saturating_sub(summary.y + if dialog.feedback.is_some() { 2 } else { 1 })
        as usize;
    let scroll = dialog.scroll.min(files.len().saturating_sub(visible));
    for (offset, file) in files.iter().enumerate().skip(scroll).take(visible) {
        let y = summary.y + 1 + (offset - scroll) as u16;
        let row = Rect::new(message.x, y, message.width, 1);
        hits.git_files.push((row, offset));
        let status = if dialog.excluded_files.contains(&file.path) {
            "○"
        } else {
            "●"
        };
        let text = format!(" {}  {}  {}", status, file.status, file.path);
        let style = if dialog.selected == offset && !dialog.editing_message {
            theme.fg(theme.text).bg(theme.bg_select)
        } else {
            theme.fg(theme.text)
        };
        frame.render_widget(Paragraph::new(text).style(style), row);
    }
    if let Some(feedback) = &dialog.feedback {
        frame.render_widget(
            Paragraph::new(format!(
                "{} · → 输出页查看完整内容",
                feedback.lines().next().unwrap_or("")
            ))
            .style(theme.fg(theme.amber)),
            Rect::new(message.x, body.bottom() - 1, message.width, 1),
        );
    }
}

fn git_diff_style(line: &str, theme: &Theme) -> Style {
    let accent = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ")
    {
        return theme.fg(theme.text_dim);
    } else if line.starts_with('+') {
        theme.sage
    } else if line.starts_with('-') {
        theme.rust
    } else if line.starts_with("@@") {
        theme.rose
    } else {
        return theme.base();
    };
    let background = match (theme.bg, accent) {
        (Color::Rgb(r, g, b), Color::Rgb(ar, ag, ab)) => {
            let blend = |base: u8, tint: u8| ((u16::from(base) * 3 + u16::from(tint)) / 4) as u8;
            Color::Rgb(blend(r, ar), blend(g, ag), blend(b, ab))
        }
        _ => theme.bg_select,
    };
    theme.fg(accent).bg(background)
}

fn render_git_diff(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &GitDialog,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let block = Block::default()
        .borders(ratatui::widgets::Borders::LEFT)
        .border_style(theme.fg(theme.hairline))
        .style(theme.base());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let content = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(1),
        inner.height,
    );
    if content.height < 3 || content.width == 0 {
        return;
    }
    let title = dialog.diff_path.as_deref().unwrap_or("");
    frame.render_widget(
        Paragraph::new(format!("差异 · {title}"))
            .style(theme.fg(theme.text).add_modifier(Modifier::BOLD)),
        Rect::new(content.x, content.y, content.width, 1),
    );
    let text_area = Rect::new(
        content.x,
        content.y + 2,
        content.width,
        content.height.saturating_sub(3),
    );
    hits.git_diff = Some(text_area);
    let lines: Vec<Line<'static>> = dialog
        .diff_text
        .lines()
        .skip(dialog.diff_scroll)
        .map(|line| {
            let style = git_diff_style(line, theme);
            Line::from(Span::styled(line.to_owned(), style)).style(style)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme.base())
            .wrap(ratatui::widgets::Wrap { trim: false }),
        text_area,
    );
    frame.render_widget(
        Paragraph::new("滚轮 / PgUp PgDn 浏览完整差异").style(theme.mute()),
        Rect::new(content.x, content.bottom() - 1, content.width, 1),
    );
}

fn git_graph_lines(log: &str, theme: &Theme) -> Vec<Line<'static>> {
    log.lines()
        .map(|line| {
            let prefix_end = line
                .char_indices()
                .find(|(_, ch)| !matches!(ch, ' ' | '*' | '|' | '/' | '\\' | '_' | '-' | '.'))
                .map_or(line.len(), |(offset, _)| offset);
            let (graph, text) = line.split_at(prefix_end);
            let colors = [theme.rose, theme.sage, theme.amber, theme.text_dim];
            let mut spans: Vec<Span<'static>> = graph
                .chars()
                .enumerate()
                .map(|(column, ch)| {
                    let glyph = match ch {
                        '*' => '●',
                        '|' => '│',
                        '_' | '-' => '─',
                        '/' => '╱',
                        '\\' => '╲',
                        ch => ch,
                    };
                    Span::styled(
                        glyph.to_string(),
                        theme.fg(colors[(column / 2) % colors.len()]),
                    )
                })
                .collect();
            if let Some((hash, subject)) = text.split_once(' ') {
                spans.push(Span::styled(hash.to_owned(), theme.mute()));
                spans.push(Span::raw(" "));
                if subject.starts_with('(')
                    && let Some(end) = subject.find(") ")
                {
                    spans.push(Span::styled(
                        subject[..end + 1].to_owned(),
                        theme.fg(theme.sage),
                    ));
                    spans.push(Span::styled(
                        subject[end + 1..].to_owned(),
                        theme.fg(theme.text),
                    ));
                } else {
                    spans.push(Span::styled(subject.to_owned(), theme.fg(theme.text)));
                }
            } else {
                spans.push(Span::styled(text.to_owned(), theme.fg(theme.text)));
            }
            Line::from(spans)
        })
        .collect()
}

fn render_tool_detail_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &ToolDetailDialog,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap::default();
    let frame_area = dialog_outer(area, 88, 14, dialog.fullscreen, dialog.minimized);
    let Some(frame_area) = frame_area else {
        return hits;
    };
    let inner = paint_dialog_frame(frame, frame_area, "工具详细信息", pointer, theme, &mut hits);
    let [_, body, hint_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(inner);

    if dialog.minimized {
        return hits;
    }
    let lines = tool_detail_lines(&dialog.tool, body.width as usize, theme);
    let visible = lines
        .into_iter()
        .skip(dialog.scroll)
        .take(body.height as usize)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(visible).style(theme.base()), body);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "↑/↓ 滚动 · Esc 关闭",
            theme.mute(),
        )))
        .style(theme.base()),
        hint_row,
    );
    hits
}

fn render_add_provider_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &AddProviderDialog,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap::default();
    let compact_height = 18;
    let Some(frame_y) = dialog_outer(
        area,
        64,
        compact_height,
        dialog.fullscreen,
        dialog.minimized,
    ) else {
        return hits;
    };
    let title = match dialog.step {
        AddProviderStep::Catalog => {
            i18n::tr("dialog.add_provider.catalog", "添加供应商 · 选择来源")
        }
        AddProviderStep::CustomId => i18n::tr("dialog.add_provider.id", "添加供应商 · 标识"),
        AddProviderStep::CustomBase => {
            i18n::tr("dialog.add_provider.base", "添加供应商 · 接口地址")
        }
        AddProviderStep::ApiKey => i18n::tr("dialog.add_provider.key", "添加供应商 · API 密钥"),
        AddProviderStep::Review => i18n::tr("dialog.add_provider.review", "添加供应商 · 确认"),
    };
    let inner = paint_dialog_frame(frame, frame_y, &title, pointer, theme, &mut hits);
    if inner.height == 0 {
        return hits;
    }
    let [_, body, hint_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(inner);
    if dialog.minimized {
        return hits;
    }

    match dialog.step {
        AddProviderStep::Catalog => {
            let filtered = dialog.filtered_catalog();
            let total = filtered.len().saturating_add(1);
            let [filter_row, list_body] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(2)]).areas(body);
            let filter_shown = if dialog.filter.is_empty() {
                "▏".to_owned()
            } else {
                format!("{}▏", dialog.filter)
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" 筛选  ", theme.mute()),
                    Span::styled(filter_shown, theme.fg(theme.text)),
                ]))
                .style(theme.base()),
                filter_row,
            );
            let visible = list_body.height.max(1) as usize;
            let offset = catalog_window_offset(dialog.selected, visible, total);
            let rows = split_n_rows(list_body, visible.min(total.saturating_sub(offset)) as u16);
            for vis in 0..rows.len() {
                let index = offset + vis;
                if index >= total {
                    break;
                }
                let rect =
                    rows.get(vis)
                        .copied()
                        .unwrap_or(Rect::new(list_body.x, list_body.y, 0, 0));
                if rect.height == 0 {
                    continue;
                }
                hits.add_provider_rows.push((rect, index));
                let hovered = pointer
                    .map(|(col, row)| contains(rect, col, row))
                    .unwrap_or(false);
                let is_selected = index == dialog.selected;
                if is_selected || hovered {
                    frame.render_widget(
                        Block::default().style(Style::default().bg(theme.bg_select)),
                        rect,
                    );
                }
                let marker = if is_selected || hovered { "❯" } else { " " };
                let (display, hint) = if index == 0 {
                    (
                        "+ 列表中没有我想要的供应商".to_owned(),
                        "自定义 OpenAI 兼容端点".to_owned(),
                    )
                } else {
                    let entry = filtered[index - 1];
                    let count = entry.models.len();
                    let count_hint = if count == 0 {
                        "需拉取模型".to_owned()
                    } else {
                        format!("{count} 个模型")
                    };
                    (
                        entry.name.clone(),
                        format!("{} · {}", count_hint, entry.format().label()),
                    )
                };
                let line = Line::from(vec![
                    Span::styled(format!(" {marker} "), theme.fg(theme.rose)),
                    Span::styled(display, theme.fg(theme.text)),
                    Span::styled("  ", theme.mute()),
                    Span::styled(hint, theme.mute()),
                ]);
                frame.render_widget(Paragraph::new(line).style(theme.base()), rect);
            }
        }
        AddProviderStep::CustomId => {
            paint_prompt_block(
                frame,
                body,
                "供应商 id",
                &dialog.custom_id,
                "crewrouter",
                "小写字母、数字、连字符或下划线",
                theme,
            );
        }
        AddProviderStep::CustomBase => {
            let preview = blora_catalog::openai_base_candidates(&dialog.custom_base)
                .into_iter()
                .find(|base| base.ends_with("/v1"));
            let hint = if dialog.custom_base.trim().is_empty() {
                "OpenAI 兼容地址，通常以 /v1 结尾，例如 https://router.bloret.net/v1".to_owned()
            } else if let Some(base) = preview {
                format!("将请求 {base}/models")
            } else {
                format!(
                    "将请求 {}/models",
                    dialog.custom_base.trim().trim_end_matches('/')
                )
            };
            paint_prompt_block(
                frame,
                body,
                "API 基址",
                &dialog.custom_base,
                "https://host/v1",
                &hint,
                theme,
            );
        }
        AddProviderStep::ApiKey => {
            let masked: String = dialog.api_key.chars().map(|_| '•').collect();
            paint_prompt_block(
                frame,
                body,
                "API key",
                &masked,
                "",
                "只保存在本机 ~/.config/blora/providers.json",
                theme,
            );
        }
        AddProviderStep::Review => {
            let [format_row, base_row, list_body] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(2),
            ])
            .areas(body);
            hits.add_provider_rows.push((format_row, 0));
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" 格式  ", theme.mute()),
                    Span::styled("← ".to_owned(), theme.fg(theme.rose)),
                    Span::styled(dialog.format.label().to_owned(), theme.fg(theme.text)),
                    Span::styled(" →".to_owned(), theme.fg(theme.rose)),
                ]))
                .style(theme.base()),
                format_row,
            );
            let base = if dialog.resolved_base.is_empty() {
                dialog.custom_base.trim()
            } else {
                dialog.resolved_base.as_str()
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" 基址  ", theme.mute()),
                    Span::styled(base.to_owned(), theme.fg(theme.text)),
                ]))
                .style(theme.base()),
                base_row,
            );
            if dialog.preview_models.is_empty() {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        " 没有列出模型。可切换格式重试，或返回修改基址。",
                        theme.fg(theme.rust),
                    )))
                    .style(theme.base()),
                    list_body,
                );
            } else {
                let shown = dialog
                    .preview_models
                    .len()
                    .clamp(1, list_body.height.max(1) as usize);
                let rows = split_n_rows(list_body, shown as u16);
                for (index, model) in dialog.preview_models.iter().take(shown).enumerate() {
                    let rect = rows.get(index).copied().unwrap_or(Rect::new(
                        list_body.x,
                        list_body.y,
                        0,
                        0,
                    ));
                    let label = if model.name == model.id {
                        model.id.clone()
                    } else {
                        format!("{}  {}", model.id, model.name)
                    };
                    frame.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled("   ", theme.mute()),
                            Span::styled(label, theme.fg(theme.text)),
                        ]))
                        .style(theme.base()),
                        rect,
                    );
                }
            }
        }
    }
    if let Some(err) = &dialog.error {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(err.clone(), theme.fg(theme.rust))))
                .style(theme.base()),
            hint_row,
        );
    } else {
        let footer = match dialog.step {
            AddProviderStep::Catalog => i18n::tr(
                "hint.add_provider.catalog",
                " 输入筛选  ·  enter 下一步  ·  esc 关闭",
            ),
            AddProviderStep::Review => i18n::tr(
                "hint.add_provider.review",
                " enter 保存  ·  ←→ 切换格式  ·  esc 返回",
            ),
            _ => i18n::tr("hint.add_provider.next", " enter 下一步  ·  esc 上一步"),
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(footer, theme.mute()))).style(theme.base()),
            hint_row,
        );
    }
    hits
}

fn catalog_window_offset(selected: usize, visible: usize, total: usize) -> usize {
    if visible == 0 || total <= visible {
        return 0;
    }
    if selected < visible {
        0
    } else {
        (selected + 1).saturating_sub(visible).min(total - visible)
    }
}

fn paint_prompt_block(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    value: &str,
    placeholder: &str,
    hint: &str,
    theme: &Theme,
) {
    let [input, hint_row] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    let (shown, style) = if value.is_empty() {
        (format!("{placeholder}▏"), theme.mute())
    } else {
        (format!("{value}▏"), theme.fg(theme.text))
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {label}  "), theme.mute()),
            Span::styled(shown, style),
        ]))
        .style(theme.base()),
        input,
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(format!(" {hint}"), theme.mute())))
            .style(theme.base()),
        hint_row,
    );
}

const THEME_TABS: [(Scheme, &str); 3] = [
    (Scheme::Auto, "自动"),
    (Scheme::Light, "浅色"),
    (Scheme::Dark, "深色"),
];

/// Color-scheme picker: three scheme sections, then the six palettes.
fn render_theme_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &ThemeDialog,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
) -> HitMap {
    let mut hits = HitMap::default();
    let palette_count = dialog.options.len().clamp(1, 12) as u16;
    let row_count = palette_count + 1;
    let compact_height = row_count + 5;
    if dialog.options.is_empty() {
        return hits;
    }
    let content_width = dialog
        .options
        .iter()
        .map(|option| option.display.width() + option.hint.width() + 14)
        .max()
        .unwrap_or(24) as u16;
    let compact_width = (content_width + 6).clamp(52, area.width.saturating_sub(4).max(52));
    let Some(frame_y) = dialog_outer(
        area,
        compact_width,
        compact_height,
        dialog.fullscreen,
        dialog.minimized,
    ) else {
        return hits;
    };

    let inner = paint_dialog_frame(frame, frame_y, "主题配色", pointer, theme, &mut hits);

    if inner.height == 0 {
        return hits;
    }
    let mut rows = split_n_rows(inner, row_count + 1);
    rows.remove(0);

    if dialog.minimized {
        return hits;
    }

    if let Some(tab_row) = rows.first().copied() {
        rows.remove(0);
        paint_theme_tabs(frame, tab_row, dialog, pointer, theme, &mut hits);
    }

    for (index, option) in dialog.options.iter().enumerate() {
        let rect = rows
            .get(index)
            .copied()
            .unwrap_or(Rect::new(inner.x, inner.y, 0, 0));
        if rect.height == 0 {
            continue;
        }
        hits.theme_rows.push((rect, index));
        let is_selected = index == dialog.selected;
        let hovered = pointer
            .map(|(col, row)| contains(rect, col, row))
            .unwrap_or(false);
        if is_selected || hovered {
            frame.render_widget(
                Block::default().style(Style::default().bg(theme.bg_select)),
                rect,
            );
        }
        let marker = if is_selected || hovered { "❯" } else { " " };
        let name_style = if is_selected {
            theme.fg(theme.text)
        } else {
            theme.fg(theme.text_dim)
        };
        let mut spans = vec![
            Span::styled(format!(" {marker} "), theme.fg(theme.rose)),
            Span::styled(option.display.clone(), name_style),
            Span::styled("  ".to_owned(), theme.mute()),
            Span::styled(option.hint.clone(), theme.mute()),
        ];
        if !option.swatches.is_empty() {
            spans.push(Span::styled("  ".to_owned(), theme.mute()));
            for color in &option.swatches {
                spans.push(Span::styled("●", Style::default().fg(*color)));
            }
        }
        frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.base()), rect);
    }
    hits
}

fn paint_theme_tabs(
    frame: &mut Frame<'_>,
    tab_row: Rect,
    dialog: &ThemeDialog,
    pointer: Option<(u16, u16)>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let tab_w = (tab_row.width / 3).max(6);
    for (index, (scheme, label)) in THEME_TABS.iter().enumerate() {
        let x = tab_row
            .x
            .saturating_add(u16::try_from(index).unwrap_or(0).saturating_mul(tab_w));
        let rect = Rect {
            x,
            y: tab_row.y,
            width: tab_w,
            height: tab_row.height.max(1),
        };
        hits.theme_tabs.push((rect, index));
        let hovered = pointer
            .map(|(col, row)| contains(rect, col, row))
            .unwrap_or(false);
        let selected = dialog.scheme == *scheme;
        if selected || hovered {
            frame.render_widget(
                Block::default().style(Style::default().bg(theme.bg_select)),
                rect,
            );
        }
        let style = if selected {
            theme.fg(theme.text).add_modifier(Modifier::BOLD)
        } else {
            theme.mute()
        };
        let marker = if selected { "▸ " } else { "  " };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(marker.to_owned(), theme.fg(theme.rose)),
                Span::styled((*label).to_owned(), style),
            ]))
            .style(theme.base()),
            rect,
        );
    }
}

fn split_n_rows(inner: Rect, count: u16) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let height = inner.height;
    let base = height / count;
    let remainder = height % count;
    let mut y = inner.y;
    (0..count)
        .map(|index| {
            let extra = u16::from(index < remainder);
            let row_height = base + extra;
            let row = Rect::new(inner.x, y, inner.width, row_height);
            y = y.saturating_add(row_height);
            row
        })
        .collect()
}

/// Even rows of the dialog inner area; the first row gets the remainder so the
/// layout is stable when the inner height is not a multiple of the row count.
fn split_dialog_rows(inner: Rect, count: u16) -> [Rect; 5] {
    let height = inner.height;
    let base = height / count;
    let remainder = height % count;
    let mut rows = [Rect::new(inner.x, inner.y, inner.width, 0); 5];
    let mut y = inner.y;
    for (index, row) in rows.iter_mut().enumerate() {
        let extra = u16::from(index < remainder as usize);
        let row_height = base + extra;
        *row = Rect::new(inner.x, y, inner.width, row_height);
        y = y.saturating_add(row_height);
    }
    rows
}

fn chrome_item(
    line: &mut Line<'static>,
    area: Rect,
    separator: &str,
    text: &str,
    style: Style,
    truncate: bool,
) -> Option<Rect> {
    if area.height == 0 || text.is_empty() {
        return None;
    }
    let separator = if line.width() == 0 { "" } else { separator };
    let offset = line.width() + separator.width();
    let available = usize::from(area.width).saturating_sub(offset);
    if available == 0 || (!truncate && text.width() > available) {
        return None;
    }
    let text = ellipsize(text, available);
    let rect = Rect::new(area.x + offset as u16, area.y, text.width() as u16, 1);
    line.spans.push(Span::styled(separator.to_owned(), style));
    line.spans.push(Span::styled(text, style));
    Some(rect)
}

fn render_composer(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let inner = inset(area);
    let prompt = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: u16::from(inner.height > 0),
    };
    let info = Rect {
        x: inner.x,
        y: inner.y.saturating_add(1),
        width: inner.width,
        height: u16::from(inner.height > 1),
    };
    if prompt.height == 0 {
        return;
    }
    let cursor_col = crate::conversation::render_composer(frame, prompt, model, theme);

    let harness = model.projection.map(|p| p.harness).unwrap_or_default();
    let external_model = model.projection.and_then(|p| p.external_model.as_deref());
    let external_label = format!(
        "{} · {} · {}",
        harness.display_name(),
        external_model.unwrap_or("—"),
        i18n::tr("harness.cli_settings", "CLI 自身模型/权限")
    );
    let model_name = if harness == blora_types::HarnessKind::Blora {
        display_model_name(model.model, model.provider)
    } else {
        &external_label
    };
    let perm = i18n::permission_name(model.permission_mode);
    let provider = if harness != blora_types::HarnessKind::Blora {
        ""
    } else if model.provider.is_empty() {
        "—"
    } else {
        model.provider
    };
    let mut line = Line::default();
    if inner.width < 60 {
        hits.toggle_approve = chrome_item(&mut line, info, "", &perm, theme.dim(), false);
        let model_hit = chrome_item(
            &mut line,
            info,
            " · ",
            model_name,
            theme.fg(theme.sage),
            true,
        );
        hits.provider_target = model_hit;
    } else {
        let model_area = Rect::new(
            info.x,
            info.y,
            info.width.saturating_sub(perm.width() as u16 + 3),
            info.height,
        );
        hits.provider_target = chrome_item(
            &mut line,
            model_area,
            "",
            model_name,
            theme.fg(theme.sage),
            true,
        );
        hits.toggle_approve = chrome_item(&mut line, info, " · ", &perm, theme.dim(), false);
        if let Some(provider_hit) =
            chrome_item(&mut line, info, " · ", provider, theme.mute(), true)
        {
            if let Some(target) = hits.provider_target.as_mut() {
                target.width = provider_hit.right().saturating_sub(target.x);
            } else {
                hits.provider_target = Some(provider_hit);
            }
        }
    }
    frame.render_widget(Paragraph::new(line).style(theme.base()), info);

    let cursor_x = prompt.x.saturating_add(2).saturating_add(cursor_col);
    if cursor_x < prompt.x.saturating_add(prompt.width)
        && !model.right_panel.is_some_and(|panel| panel.focused)
    {
        frame.set_cursor_position(Position::new(cursor_x, prompt.y));
    }
}

fn render_status(frame: &mut Frame<'_>, area: Rect, model: &FrameModel<'_>, theme: &Theme) {
    let inner = inset(area);
    let left = workspace_status(model.workspace, model.status, inner.width as usize);
    frame.render_widget(Paragraph::new(left).style(theme.mute()), inner);
}

fn render_hints(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let inner = inset(area);
    let items: &[(&str, &str, HintAction)] = if slash::is_open(model.input) {
        &[
            ("tab", "complete", HintAction::Complete),
            ("↑↓", "select", HintAction::Run),
            ("⏎", "run", HintAction::Run),
            ("esc", "close", HintAction::Close),
        ]
    } else if !model.pending.is_empty() && model.input.is_empty() {
        &[
            ("y", "allow", HintAction::Allow),
            ("n", "deny", HintAction::Deny),
            ("esc", "quit", HintAction::Quit),
        ]
    } else {
        &[
            ("⏎", "send", HintAction::Send),
            ("/", "commands", HintAction::Commands),
            ("‹ ›", "session", HintAction::SessionNext),
            ("^n", "new", HintAction::New),
            ("^c", "quit", HintAction::Quit),
        ]
    };
    let mut line = Line::default();
    for (key, label, action) in items {
        let text = if matches!(action, HintAction::Send) && model.running {
            if inner.width < 60 {
                i18n::tr("hint.queue.compact", "⏎ 排队")
            } else {
                i18n::tr("hint.queue", "⏎ queue")
            }
        } else if inner.width < 60 {
            match action {
                HintAction::Send => "⏎ 发送",
                HintAction::Commands => "/ 命令",
                HintAction::SessionNext => "› 会话",
                HintAction::New => "+ 新建",
                HintAction::Complete => "Tab 补全",
                HintAction::Run if *key == "↑↓" => continue,
                HintAction::Run => "⏎ 执行",
                HintAction::Close => "Esc 关闭",
                HintAction::Allow => "y 允许",
                HintAction::Deny => "n 拒绝",
                HintAction::Quit => "退出",
            }
            .to_owned()
        } else {
            format!("{key} {label}")
        };
        if let Some(rect) = chrome_item(&mut line, inner, "  ", &text, theme.dim(), false) {
            hits.hints.push((rect, *action));
        }
    }
    frame.render_widget(Paragraph::new(line).style(theme.base()), inner);
}

pub(crate) fn visible_input(input: &str, width: usize) -> (String, u16) {
    if width == 0 {
        return (String::new(), 0);
    }
    let full = input.width();
    if full <= width {
        return (input.to_owned(), u16::try_from(full).unwrap_or(0));
    }
    let mut acc = 0usize;
    let mut start = input.len();
    for (idx, ch) in input.char_indices().rev() {
        let char_width = ch.width().unwrap_or(0);
        if acc + char_width > width {
            break;
        }
        acc += char_width;
        start = idx;
    }
    let slice = &input[start..];
    (
        slice.to_owned(),
        u16::try_from(slice.width().min(width)).unwrap_or(0),
    )
}

#[must_use]
pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for para in text.split('\n') {
        if para.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut current_w = 0usize;
        for ch in para.chars() {
            let w = ch.width().unwrap_or(0);
            if current_w + w > width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_w = 0;
            }
            current.push(ch);
            current_w += w;
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

const CONTEXT_BAR_CELLS: usize = 10;

fn context_usage_line(used: u64, window: u64, theme: &Theme) -> Line<'static> {
    let window = window.max(1);
    let ratio = (used as f64 / window as f64).clamp(0.0, 1.0);
    let exact = ratio * CONTEXT_BAR_CELLS as f64;
    let filled = exact.floor() as usize;
    let filled = filled.min(CONTEXT_BAR_CELLS);
    let has_partial = exact.fract() >= 0.05 && filled < CONTEXT_BAR_CELLS;
    let empty = CONTEXT_BAR_CELLS - filled - usize::from(has_partial);
    let pct = (ratio * 100.0).round() as u16;
    let fill = "■".repeat(filled);
    let partial = if has_partial { "▣" } else { "" };
    let rest = "▢".repeat(empty);
    let color = if pct >= 90 {
        theme.rust
    } else if pct >= 85 {
        theme.amber
    } else {
        theme.sage
    };
    Line::from(vec![
        Span::styled(fill, theme.fg(color)),
        Span::styled(partial.to_owned(), theme.fg(color)),
        Span::styled(rest, theme.mute()),
        Span::styled(format!(" {pct}%"), theme.mute()),
    ])
}

pub fn fmt_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 10_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else if n < 1_000_000 {
        format!("{}K", n / 1_000)
    } else if n < 10_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else {
        format!("{}M", n / 1_000_000)
    }
}

fn ellipsize(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if text.width() <= max {
        return text.to_owned();
    }
    if max <= 1 {
        return "…".to_owned();
    }
    let mut acc = 0usize;
    let mut out = String::new();
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if acc + w + 1 > max {
            break;
        }
        out.push(ch);
        acc += w;
    }
    out.push('…');
    out
}

fn pad_right(text: &str, width: usize) -> String {
    let w = text.width();
    if w >= width {
        ellipsize(text, width)
    } else {
        format!("{text}{}", " ".repeat(width - w))
    }
}

fn pretty_route_id(id: &str) -> String {
    if id.eq_ignore_ascii_case("blora")
        || id.eq_ignore_ascii_case("passport")
        || id.eq_ignore_ascii_case("bloret-passport")
    {
        return "Bloret PassPort".to_owned();
    }
    blora_catalog::find_saved(id)
        .map(|saved| saved.name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.to_owned())
}

pub(crate) fn spinner(tick: u64) -> char {
    let n = SPINNER.len();
    if n == 0 {
        return '✦';
    }
    if n == 1 {
        return SPINNER[0];
    }
    let period = 2 * (n - 1);
    let t = (tick as usize) % period;
    let index = if t < n { t } else { period - t };
    SPINNER[index]
}

fn sanitize_title(text: &str) -> String {
    text.chars()
        .filter(|ch| *ch != '\u{1b}' && *ch != '\u{07}' && *ch != '\n' && *ch != '\r')
        .collect()
}

fn display_model_name<'a>(model: &'a str, provider: &str) -> &'a str {
    if !model.is_empty() {
        return model;
    }
    if provider.eq_ignore_ascii_case("blora")
        || provider.eq_ignore_ascii_case("passport")
        || provider.eq_ignore_ascii_case("bloret-passport")
        || provider.eq_ignore_ascii_case("bloret passport")
    {
        "blora"
    } else {
        "—"
    }
}

#[allow(dead_code)]
fn dialog_rect(area: Rect, width: u16, height: u16) -> Option<Rect> {
    if width == 0 || height == 0 || width > area.width || height > area.height {
        return None;
    }
    Some(Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    })
}

fn mode_label(mode: &str) -> String {
    let mut chars = mode.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Code".to_owned(),
    }
}

/// Terminal window title. While a run is in progress the star spinner
/// ping-pongs through [`SPINNER`].
#[must_use]
pub(crate) fn window_title(running: bool, tick: u64, mode: &str, label: &str) -> String {
    let mode = mode_label(mode);
    let label = ellipsize(&sanitize_title(label.trim()), 32);
    if running {
        if label.is_empty() {
            format!("{} Blora {mode}", spinner(tick))
        } else {
            format!("{} Blora {mode} · {label}", spinner(tick))
        }
    } else if label.is_empty() {
        format!("Blora {mode}")
    } else {
        format!("Blora {mode} · {label}")
    }
}

#[must_use]
pub(crate) fn title_label(
    projection: Option<&SessionProjection>,
    session_title: Option<&str>,
) -> String {
    projection
        .and_then(|projection| projection.session.as_ref())
        .and_then(|session| session.title.as_deref())
        .or(session_title)
        .map(str::trim)
        .filter(|text| !text.is_empty() && *text != "tui")
        .map(|text| ellipsize(text, 32))
        .unwrap_or_default()
}

fn line_count(text: &str) -> usize {
    text.lines().count().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Palette;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn welcome_adapts_to_small_and_large_canvases() {
        let theme = Theme::current();
        for width in [0, 18, 22, 36, 80, 120] {
            for height in [0, 1, 3, 6, 20] {
                let lines = welcome_lines(width, height, &theme);
                assert!(lines.len() <= height);
                assert!(lines.iter().all(|line| line.width() <= width));
                if width >= 18 && height >= 3 {
                    assert!(lines.iter().any(|line| line.to_string().contains('/')));
                }
            }
        }
        let lines = welcome_lines(80, 20, &theme);
        assert!(lines.iter().any(|line| line.to_string().contains("Ctrl+N")));
    }

    #[test]
    fn composer_feedback_and_workspace_labels_fit_available_width() {
        for width in [0, 1, 18, 22, 80] {
            for imagine in [false, true] {
                for running in [false, true] {
                    assert!(composer_placeholder(imagine, running, width).width() <= width);
                }
            }
            assert!(workspace_status(Path::new("/data/中文项目"), "", width).width() <= width);
        }
        assert!(composer_placeholder(false, true, 80).contains("Enter"));
        assert_eq!(
            workspace_status(Path::new("/data/project"), "", 80),
            "project"
        );
        assert_eq!(workspace_status(Path::new("/"), "", 80), "/");
        assert_eq!(
            workspace_status(Path::new("/data/project"), "ready", 80),
            "ready"
        );
    }

    #[test]
    fn harness_disabled_keys_and_minimize() {
        use crossterm::event::KeyCode;
        let kinds = blora_types::HarnessKind::all();
        assert_eq!(kinds.len(), 9);
        let mut dialog = HarnessDialog {
            selected: 0,
            available: vec![true; kinds.len()],
            fullscreen: false,
            minimized: false,
        };
        assert_eq!(dialog.select(kinds.len()), None);
        dialog.key(KeyCode::Up);
        assert_eq!(dialog.selected, 0);
        for (index, kind) in kinds.into_iter().enumerate() {
            assert_eq!(dialog.selected, index);
            dialog.available[index] = false;
            assert_eq!(dialog.select(index), None);
            assert_eq!(dialog.key(KeyCode::Enter), None);
            dialog.available[index] = true;
            assert_eq!(dialog.key(KeyCode::Enter), Some(kind));
            dialog.key(KeyCode::Down);
        }
        assert_eq!(dialog.selected, kinds.len() - 1);
        dialog.key(KeyCode::Down);
        assert_eq!(dialog.selected, kinds.len() - 1);
        dialog.minimized = true;
        assert_eq!(dialog.select(0), None);
        dialog.key(KeyCode::Up);
        assert_eq!(dialog.key(KeyCode::Enter), None);
        assert!(!dialog.minimized);
        assert_eq!(dialog.selected, kinds.len() - 1);
        for index in (0..kinds.len() - 1).rev() {
            dialog.key(KeyCode::Up);
            assert_eq!(dialog.selected, index);
        }
    }

    fn frame_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        (0..area.height)
            .map(|y| {
                let mut line = String::new();
                let mut skip = 0usize;
                for x in 0..area.width {
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    let symbol = buffer[(x, y)].symbol();
                    skip = symbol.width().saturating_sub(1);
                    line.push_str(symbol);
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn question_projection() -> SessionProjection {
        use blora_events::{
            AssistantReasoning, ContextCompactionCompleted, EventEnvelope, InputRequested,
            KnownPayload, NewEvent, SessionCreated,
        };
        let id = blora_types::SessionId::generate();
        let payloads = vec![
            KnownPayload::SessionCreated(SessionCreated {
                title: Some("q".to_owned()),
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Agent,
                parent_session_id: None,
            }),
            KnownPayload::AssistantReasoning(AssistantReasoning {
                text: "HIDDEN-THOUGHT about the plan".to_owned(),
            }),
            KnownPayload::ContextCompactionCompleted(ContextCompactionCompleted {
                summary: "s".to_owned(),
                preserve_from_sequence: None,
                tokens_before: 0,
                tokens_after: 0,
            }),
            KnownPayload::InputRequested(InputRequested {
                request_id: "ask_1".to_owned(),
                question: "Which region?".to_owned(),
                options: vec!["EU".to_owned(), "US".to_owned()],
                call_id: None,
                questions: Vec::new(),
            }),
        ];
        let events: Vec<EventEnvelope> = payloads
            .into_iter()
            .enumerate()
            .map(|(i, payload)| {
                EventEnvelope::from_new(
                    NewEvent::new(id.clone(), payload),
                    i as u64 + 1,
                    &blora_types::SystemClock,
                )
                .unwrap()
            })
            .collect();
        blora_session::rebuild(&events).unwrap()
    }

    #[test]
    fn question_bar_reasoning_fold_and_compaction_badge() {
        let projection = question_projection();
        assert!(projection.pending_input.is_some());
        let mut model = FrameModel {
            dialog_stack: &[],
            workspace: Path::new("/tmp"),
            sessions: &[],
            index: 0,
            sidebar: &SidebarState::default(),
            right_panel: None,
            projection: Some(&projection),
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
            mentions: None,
            search: None,
            hide_tools: false,
            message_roles: true,
            scroll: 0,
            permission_mode: PermissionMode::Ask,
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
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut cache = TranscriptCache::default();
        terminal
            .draw(|frame| {
                draw(frame, &model, &mut cache);
            })
            .unwrap();
        let text = frame_text(&terminal);
        assert!(text.contains("Which region?"), "{text}");
        assert!(text.contains("1. EU"), "{text}");
        assert!(
            !text.contains("数字键选择选项"),
            "the composer no longer answers questions:\n{text}"
        );
        assert!(text.contains("思考过程"), "reasoning is summarised");
        assert!(
            !text.contains("HIDDEN-THOUGHT"),
            "reasoning is folded by default"
        );
        assert!(text.contains("⟲1"), "compaction count is shown");
        assert!(toggle_reasoning());
        model.input = "x";
        terminal
            .draw(|frame| {
                draw(frame, &model, &mut cache);
            })
            .unwrap();
        assert!(frame_text(&terminal).contains("HIDDEN-THOUGHT"));
        assert!(!toggle_reasoning());
    }

    #[test]
    fn question_dialog_shows_types_other_input_and_files() {
        let dir = std::env::temp_dir().join(format!("blora-ask-view-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("note.txt"), "hi").unwrap();
        let questions = vec![
            blora_events::AskedQuestion {
                id: "q1".into(),
                kind: "single".into(),
                question: "用哪个？".into(),
                header: Some("数据库".into()),
                options: vec![
                    blora_events::QuestionChoice {
                        label: "SQLite".into(),
                        description: None,
                        other: false,
                    },
                    blora_events::QuestionChoice {
                        label: "其他".into(),
                        description: None,
                        other: true,
                    },
                ],
            },
            blora_events::AskedQuestion {
                id: "q2".into(),
                kind: "boolean".into(),
                question: "继续吗？".into(),
                header: Some("继续".into()),
                options: vec![
                    blora_events::QuestionChoice {
                        label: "是".into(),
                        description: None,
                        other: false,
                    },
                    blora_events::QuestionChoice {
                        label: "否".into(),
                        description: None,
                        other: false,
                    },
                ],
            },
            blora_events::AskedQuestion {
                id: "q3".into(),
                kind: "file".into(),
                question: "读哪个文件？".into(),
                header: Some("文件".into()),
                options: Vec::new(),
            },
        ];
        let pending = blora_session::PendingInput {
            request_id: "ask_1".into(),
            question: "用哪个？".into(),
            options: Vec::new(),
            questions,
        };
        let mut form = crate::question::QuestionForm::open(&pending, &dir);
        form.on_key(crossterm::event::KeyCode::Down, &dir);
        form.on_key(crossterm::event::KeyCode::Char(' '), &dir);
        form.on_key(crossterm::event::KeyCode::Char('T'), &dir);
        form.on_key(crossterm::event::KeyCode::Char('u'), &dir);
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut render = |form: &crate::question::QuestionForm| {
            let stack = [DialogId::Question];
            let model = FrameModel {
                dialog_stack: &stack,
                workspace: &dir,
                sessions: &[],
                index: 0,
                sidebar: &SidebarState::default(),
                right_panel: None,
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
                mentions: None,
                search: None,
                hide_tools: false,
                message_roles: true,
                scroll: 0,
                permission_mode: PermissionMode::Ask,
                model: "mock",
                provider: "mock",
                mode_menu: None,
                project_picker: None,
                folder_picker: None,
                question_dialog: Some(form),
                session_picker: None,
                git_dialog: None,
                git_status: None,
                user_label: "you",
                running: false,
                tick: 0,
                pointer: None,
                text_selection: None,
            };
            let mut cache = TranscriptCache::default();
            terminal
                .draw(|frame| {
                    draw(frame, &model, &mut cache);
                })
                .unwrap();
            frame_text(&terminal)
        };
        let text = render(&form);
        assert!(text.contains("Blora 提问"), "{text}");
        assert!(text.contains("数据库"), "{text}");
        assert!(text.contains("其他"), "{text}");
        assert!(text.contains("Tu"), "{text}");
        assert!(text.contains("▌"), "{text}");
        assert!(text.contains('['), "{text}");
        let tabs_at = text.find("继续").expect("sibling title");
        let prompt_at = text.find("用哪个").expect("prompt");
        assert!(
            tabs_at < prompt_at && text.find("文件").unwrap() < prompt_at,
            "titles stay on the top row:\n{text}"
        );
        form.focus = 2;
        let text = render(&form);
        assert!(text.contains("note.txt"), "{text}");
        assert!(text.contains("sub/"), "{text}");
        assert!(text.find("数据库").unwrap() < text.find("读哪个").unwrap());
        assert!(
            !text.contains(" .."),
            "parent stays inside the workspace:\n{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn harness_mobile_hits_disabled_gray_and_controls() {
        let mut model = FrameModel {
            dialog_stack: &[],
            workspace: Path::new("/tmp"),
            sessions: &[],
            index: 0,
            sidebar: &SidebarState::default(),
            right_panel: None,
            projection: None,
            pending: &[],
            input: "手机终端输入一段很长的中文消息用于验证光标",
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
            mentions: None,
            search: None,
            hide_tools: false,
            message_roles: true,
            scroll: 0,
            permission_mode: PermissionMode::Ask,
            model: "a-very-long-model-name-that-exceeds-the-phone-width",
            provider: "provider",
            mode_menu: None,
            project_picker: None,
            folder_picker: None,
            question_dialog: None,
            session_picker: None,
            git_dialog: None,
            git_status: None,
            user_label: "you",
            running: false,
            tick: 0,
            pointer: None,
            text_selection: None,
        };
        let mut projection = SessionProjection::new();
        projection.harness = blora_types::HarnessKind::ClaudeCode;
        model.projection = Some(&projection);
        for (width, height) in [(20, 6), (24, 10), (40, 24), (100, 30)] {
            model.dialog_stack = &[];
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut cache = TranscriptCache::default();
            let mut hits = HitMap::default();
            terminal
                .draw(|frame| hits = draw(frame, &model, &mut cache))
                .unwrap();
            let rect = hits.harness.expect("visible harness entry");
            assert!(rect.width > 0);
            assert_eq!(hits.hit(rect.x, rect.y), Some(Hit::Harness));
            for selected in 0..blora_types::HarnessKind::all().len() {
                for (minimized, fullscreen) in [(false, false), (true, false), (false, true)] {
                    let mut dialog = HarnessDialog {
                        selected,
                        available: blora_types::HarnessKind::all()
                            .iter()
                            .enumerate()
                            .map(|(index, _)| index != selected && index != 1)
                            .collect(),
                        fullscreen,
                        minimized,
                    };
                    let mut local = model.clone();
                    local.dialog_stack = &[DialogId::Settings, DialogId::Harness];
                    local.harness_dialog = Some(&dialog);
                    terminal
                        .draw(|frame| hits = draw(frame, &local, &mut cache))
                        .unwrap();
                    assert!(hits.modal);
                    assert!(hits.traffic_lights.iter().all(Option::is_some));
                    if minimized {
                        assert!(hits.harness_rows.is_empty());
                    } else {
                        assert!(
                            hits.harness_rows
                                .iter()
                                .any(|(_, index)| *index == selected)
                        );
                        if width == 100 && height == 30 && !fullscreen {
                            assert_eq!(
                                hits.harness_rows.len(),
                                blora_types::HarnessKind::all().len()
                            );
                        }
                        if width == 24
                            && height == 10
                            && selected == blora_types::HarnessKind::all().len() - 1
                        {
                            assert!(hits.harness_rows[0].1 > 0);
                        }
                        for (rect, index) in &hits.harness_rows {
                            assert_eq!(hits.hit(rect.x, rect.y), Some(Hit::HarnessRow(*index)));
                            if !dialog.available[*index] {
                                assert_eq!(
                                    terminal.backend().buffer()[(rect.x, rect.y)].fg,
                                    Theme::current().mute().fg.unwrap()
                                );
                            }
                        }
                    }
                    let disabled = dialog.select(1);
                    assert!(disabled.is_none());
                    assert_eq!(dialog.select(selected), None);
                    dialog.available[selected] = true;
                    if !minimized {
                        assert_eq!(
                            dialog.select(selected),
                            Some(blora_types::HarnessKind::all()[selected])
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn permission_keyboard_confirm_cancel_and_minimize() {
        use crossterm::event::KeyCode;
        for (index, mode) in PermissionMode::ALL.iter().enumerate() {
            let mut dialog = PermissionDialog::new(*mode);
            assert_eq!(dialog.selected, index);
            if *mode == PermissionMode::Yolo {
                assert_eq!(dialog.key(KeyCode::Enter), PermissionAction::None);
                assert!(dialog.confirm_bypass);
                assert_eq!(dialog.key(KeyCode::Enter), PermissionAction::None);
                assert_eq!(dialog.select(index), PermissionAction::None);
                assert_eq!(
                    dialog.key(KeyCode::Char('y')),
                    PermissionAction::Apply(*mode)
                );
            } else {
                assert_eq!(dialog.key(KeyCode::Enter), PermissionAction::Apply(*mode));
            }
            assert_eq!(dialog.key(KeyCode::Esc), PermissionAction::Close);
            dialog.minimized = true;
            assert_eq!(dialog.key(KeyCode::Down), PermissionAction::None);
            assert_eq!(dialog.selected, index);
            assert_eq!(dialog.key(KeyCode::Enter), PermissionAction::None);
            assert!(!dialog.minimized);
        }
        let mut dialog = PermissionDialog::new(PermissionMode::Ask);
        dialog.key(KeyCode::Up);
        assert_eq!(dialog.selected, 0);
        for _ in 0..10 {
            dialog.key(KeyCode::Down);
        }
        assert_eq!(dialog.selected, PermissionMode::ALL.len() - 1);
    }

    #[test]
    fn permission_window_rendering_and_hits_at_mobile_sizes() {
        let mut model = FrameModel {
            dialog_stack: &[],
            workspace: Path::new("/tmp"),
            sessions: &[],
            index: 0,
            sidebar: &SidebarState::default(),
            right_panel: None,
            projection: None,
            pending: &[],
            input: "手机终端输入一段很长的中文消息用于验证光标",
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
            mentions: None,
            search: None,
            hide_tools: false,
            message_roles: true,
            scroll: 0,
            permission_mode: PermissionMode::Ask,
            model: "a-very-long-model-name-that-exceeds-the-phone-width",
            provider: "provider",
            mode_menu: None,
            project_picker: None,
            folder_picker: None,
            question_dialog: None,
            session_picker: None,
            git_dialog: None,
            git_status: None,
            user_label: "you",
            running: false,
            tick: 0,
            pointer: None,
            text_selection: None,
        };
        for (width, height) in [(20, 10), (24, 10), (36, 12), (40, 24), (100, 30)] {
            for mode in PermissionMode::ALL {
                let mut dialog = PermissionDialog::new(mode);
                model.permission_mode = mode;
                model.dialog_stack = &[];
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let mut cache = TranscriptCache::default();
                let mut footer_hits = HitMap::default();
                terminal
                    .draw(|frame| footer_hits = draw(frame, &model, &mut cache))
                    .unwrap();
                let rect = footer_hits
                    .toggle_approve
                    .expect("visible permission footer");
                assert_eq!(footer_hits.hit(rect.x, rect.y), Some(Hit::ToggleApprove));
                let text: String = (rect.x..rect.right())
                    .map(|x| terminal.backend().buffer()[(x, rect.y)].symbol())
                    .collect();
                assert!(
                    text.replace(' ', "")
                        .contains(&i18n::permission_name(mode).replace(' ', "")),
                    "{text}"
                );
                model.dialog_stack = &[DialogId::Permission];
                for (minimized, fullscreen) in [(false, false), (true, false), (false, true)] {
                    dialog.minimized = minimized;
                    dialog.fullscreen = fullscreen;
                    let mut model = model.clone();
                    model.permission_dialog = Some(&dialog);
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    let mut cache = TranscriptCache::default();
                    let mut hits = HitMap::default();
                    terminal
                        .draw(|frame| hits = draw(frame, &model, &mut cache))
                        .unwrap();
                    assert!(hits.modal);
                    assert!(hits.traffic_lights.iter().all(Option::is_some));
                    for (index, hit) in [
                        Hit::TrafficClose,
                        Hit::TrafficMinimize,
                        Hit::TrafficOpenBrowser,
                    ]
                    .iter()
                    .enumerate()
                    {
                        let rect = hits.traffic_lights[index].unwrap();
                        assert_eq!(hits.hit(rect.x, rect.y).as_ref(), Some(hit));
                    }
                    if minimized {
                        assert!(hits.permission_rows.is_empty());
                    } else {
                        let (rect, index) = hits
                            .permission_rows
                            .iter()
                            .find(|(_, index)| *index == dialog.selected)
                            .unwrap();
                        assert_eq!(hits.hit(rect.x, rect.y), Some(Hit::PermissionRow(*index)));
                        let row: String = (rect.x..rect.right())
                            .map(|x| terminal.backend().buffer()[(x, rect.y)].symbol())
                            .collect();
                        assert!(
                            row.replace(' ', "")
                                .contains(&i18n::permission_name(mode).replace(' ', "")),
                            "{row}"
                        );
                        if rect.width >= i18n::permission_name(mode).width() as u16 + 7 {
                            assert!(row.contains("当"), "current marker missing: {row}");
                        }
                        for (rect, _) in &hits.permission_rows {
                            assert!(rect.right() <= width && rect.bottom() <= height);
                        }
                    }
                    assert_eq!(hits.hit(0, 0), None);
                    model.permission_dialog = None;
                }
            }
        }
    }

    #[test]
    fn bypass_warning_rejects_enter_and_mouse_selection() {
        let mut dialog = PermissionDialog::bypass_warning();
        assert_eq!(
            dialog.key(crossterm::event::KeyCode::Enter),
            PermissionAction::None
        );
        assert_eq!(
            dialog.key(crossterm::event::KeyCode::Char('n')),
            PermissionAction::Close
        );
        assert_eq!(dialog.select(0), PermissionAction::None);
    }

    #[test]
    fn diff_backgrounds_distinguish_added_removed_and_context() {
        for theme in [Theme::dark(), Theme::light()] {
            let added = git_diff_style("+new", &theme);
            let removed = git_diff_style("-old", &theme);
            assert_ne!(added.bg, removed.bg);
            assert_ne!(added.bg, Some(theme.bg));
            assert_ne!(removed.bg, Some(theme.bg));
            assert_eq!(git_diff_style(" context", &theme), theme.base());
            assert_eq!(
                git_diff_style("+++ b/file", &theme),
                theme.fg(theme.text_dim)
            );
        }
    }

    #[test]
    fn git_graph_keeps_connectors_and_does_not_invent_nodes() {
        let theme = Theme::dark();
        let log = "*   abc123 (HEAD -> main) merge\n|\\  \n| * def456 (feature) change\n|/  \n* fed321 root";
        let lines = git_graph_lines(log, &theme);
        let text: Vec<String> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect();
        assert_eq!(text[0], "●   abc123 (HEAD -> main) merge");
        assert_eq!(text[1], "│╲  ");
        assert_eq!(text[2], "│ ● def456 (feature) change");
        assert_eq!(text[3], "│╱  ");
        assert_eq!(text[4], "● fed321 root");
        assert_eq!(
            text.iter()
                .map(|line| line.matches('●').count())
                .sum::<usize>(),
            3
        );
    }

    #[test]
    fn idle_context_bar_fills_by_usage() {
        let theme = Theme::current();
        let empty = context_usage_line(0, 100_000, &theme);
        let empty_text: String = empty
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(empty_text.contains("▢"), "{empty_text}");
        assert!(empty_text.contains("0%"), "{empty_text}");
        let full = context_usage_line(100_000, 100_000, &theme);
        let full_text: String = full
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(full_text.contains("■■■■■■■■■■"), "{full_text}");
        assert!(full_text.contains("100%"), "{full_text}");
        let mid = context_usage_line(35_000, 100_000, &theme);
        let mid_text: String = mid.spans.iter().map(|span| span.content.as_ref()).collect();
        assert!(mid_text.contains("▣"), "{mid_text}");
        assert!(mid_text.contains("▢"), "{mid_text}");
    }

    #[test]
    fn context_dialog_has_current_overview_and_model_pages() {
        use chrono::{Duration, TimeZone, Utc};
        let today = Utc::now().date_naive();
        let previous = today.checked_sub_signed(Duration::days(1)).unwrap_or(today);
        let activity = blora_storage::UsageActivity {
            samples: vec![blora_storage::UsageSample {
                day: previous,
                model: "Fable 5.1".to_owned(),
                session_id: "s".to_owned(),
                input_tokens: 1_000,
                output_tokens: 2_000,
                cached_tokens: 30,
            }],
            sessions: vec![blora_storage::SessionSpan {
                id: "s".to_owned(),
                created_at: Utc.from_utc_datetime(&previous.and_hms_opt(0, 0, 0).unwrap()),
                updated_at: Utc.from_utc_datetime(&today.and_hms_opt(4, 0, 0).unwrap()),
            }],
        };
        let mut projection = SessionProjection::default();
        projection.input_tokens = 1_000;
        projection.output_tokens = 250;
        let mut dialog = ContextDialog {
            scroll: 0,
            page: 0,
            range: 0,
            fullscreen: false,
            minimized: false,
            activity,
        };
        let theme = Theme::dark();
        let page0 = context_page_text(&projection, &dialog, &theme);
        assert!(page0.contains("当前上下文"), "{page0}");
        assert!(page0.contains("总览"), "{page0}");
        assert!(page0.contains("模型"), "{page0}");
        assert!(page0.contains("翻页"), "{page0}");
        dialog.page = 1;
        let page1 = context_page_text(&projection, &dialog, &theme);
        assert!(page1.contains("常用模型"), "{page1}");
        assert!(page1.contains("Fable 5.1"), "{page1}");
        assert!(page1.contains("近 7 天"), "{page1}");
        assert!(page1.contains("少"), "{page1}");
        dialog.page = 2;
        let (page2, hits) = context_page(&projection, &dialog, &theme);
        assert!(page2.contains("每日 Token"), "{page2}");
        assert!(page2.contains("Fable 5.1"), "{page2}");
        assert!(page2.contains("缓存读取"), "{page2}");
        assert_eq!(hits.context_tabs.len(), 3);
        assert_eq!(hits.context_ranges.len(), 3);
        let (tab, index) = &hits.context_tabs[1];
        assert_eq!(*index, 1);
        assert_eq!(hits.hit(tab.x, tab.y), Some(Hit::ContextTab(1)));
        let (range, range_index) = &hits.context_ranges[2];
        assert_eq!(*range_index, 2);
        assert_eq!(hits.hit(range.x, range.y), Some(Hit::ContextRange(2)));
        dialog.cycle_page(1);
        assert_eq!(dialog.page, 0);
        dialog.cycle_page(-1);
        assert_eq!(dialog.page, 2);
        dialog.cycle_range();
        assert_eq!(dialog.range, 1);
        dialog.set_page(0);
        dialog.cycle_range();
        assert_eq!(dialog.range, 1);
    }

    fn context_page_text(
        projection: &SessionProjection,
        dialog: &ContextDialog,
        theme: &Theme,
    ) -> String {
        context_page(projection, dialog, theme).0
    }

    fn context_page(
        projection: &SessionProjection,
        dialog: &ContextDialog,
        theme: &Theme,
    ) -> (String, HitMap) {
        let mut terminal = Terminal::new(TestBackend::new(110, 36)).unwrap();
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| {
                hits = render_context_dialog(
                    frame,
                    frame.area(),
                    Some(projection),
                    dialog,
                    None,
                    theme,
                );
            })
            .unwrap();
        (frame_text(&terminal), hits)
    }

    #[test]
    fn empty_passport_model_displays_blora_not_mock() {
        assert_eq!(display_model_name("", "Bloret PassPort"), "blora");
        assert_eq!(display_model_name("", "blora"), "blora");
        assert_eq!(display_model_name("", "crewrouter"), "—");
        assert_eq!(display_model_name("fusion", "crewrouter"), "fusion");
    }

    #[test]
    fn transcript_window_pins_newest_lines() {
        let lines: Vec<Line> = (0..10).map(|i| Line::from(i.to_string())).collect();
        let visible = transcript_window(&lines, None, 4, 0);
        assert_eq!(
            visible.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec!["6", "7", "8", "9"]
        );
        let older = transcript_window(&lines, None, 4, 3);
        assert_eq!(
            older.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec!["3", "4", "5", "6"]
        );
        let with_spinner = transcript_window(&lines, Some(Line::from("spin")), 4, 0);
        assert_eq!(
            with_spinner
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["7", "8", "9", "spin"]
        );
    }

    #[test]
    fn git_paths_decode_quotes_octal_and_rename_pairs() {
        let files = git_changed_files(
            "## main\n D \"node_modules/a file.txt\"\n?? \"\\344\\270\\255\\346\\226\\207.txt\"\nR  \"old -> name.txt\" -> \"new name.txt\"\n?? plain -> name.txt\n?? \"tab\\tquote\\\"slash\\\\.txt\"",
        );
        assert_eq!(files[0].path, "node_modules/a file.txt");
        assert_eq!(files[1].path, "中文.txt");
        assert_eq!(files[2].original_path.as_deref(), Some("old -> name.txt"));
        assert_eq!(files[2].path, "new name.txt");
        assert_eq!(files[3].path, "plain -> name.txt");
        assert!(files[3].original_path.is_none());
        assert_eq!(files[4].path, "tab\tquote\"slash\\.txt");
    }

    #[test]
    fn git_output_page_wraps_and_scrolls_to_last_error_line() {
        for (width, height) in [(110, 30), (48, 18)] {
            let area = Rect::new(0, 0, width, height);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let feedback = format!(
                "execution error: {}\n{}\nEND_OF_ERROR",
                "long/path/".repeat(40),
                "details\n".repeat(50)
            );
            let mut dialog = GitDialog {
                info: GitStatusInfo {
                    branch: String::new(),
                    added: 0,
                    removed: 0,
                    ahead: 0,
                    behind: 0,
                    stashes: 0,
                    clean: true,
                    raw: String::new(),
                    diff: String::new(),
                    log: String::new(),
                },
                page: 2,
                selected: 0,
                message: String::new(),
                editing_message: false,
                generating_message: false,
                syncing: false,
                excluded_files: Default::default(),
                diff_path: None,
                diff_text: String::new(),
                diff_scroll: 0,
                scroll: 0,
                feedback: Some(feedback.clone()),
                fullscreen: false,
                minimized: false,
            };
            let wrapped = git_feedback_lines(Some(&feedback), width as usize);
            assert_eq!(wrapped.join(""), feedback.replace('\n', ""));
            terminal
                .draw(|frame| {
                    render_git_dialog(frame, area, &dialog, None, &Theme::current(), 0);
                })
                .unwrap();
            let rendered: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(rendered.contains("execution error:"));
            assert!(!rendered.contains("END_OF_ERROR"));
            dialog.scroll = usize::MAX;
            terminal
                .draw(|frame| {
                    render_git_dialog(frame, area, &dialog, None, &Theme::current(), 0);
                })
                .unwrap();
            let rendered: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(rendered.contains("END_OF_ERROR"));
        }
        assert_eq!(git_feedback_lines(None, 80), vec!["暂无操作输出"]);
    }

    #[test]
    fn git_commit_page_renders_files_and_click_targets() {
        let area = Rect::new(0, 0, 110, 30);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        let dialog = GitDialog {
            info: GitStatusInfo {
                branch: "main".into(),
                added: 2,
                removed: 0,
                ahead: 0,
                behind: 0,
                stashes: 0,
                clean: false,
                raw: "## main\nM  src/main.rs\n M src/view.rs\n?? notes.txt".into(),
                diff: String::new(),
                log: String::new(),
            },
            page: 0,
            selected: 0,
            message: "修复界面".into(),
            editing_message: false,
            generating_message: false,
            syncing: false,
            excluded_files: Default::default(),
            diff_path: None,
            diff_text: String::new(),
            diff_scroll: 0,
            scroll: 0,
            feedback: None,
            fullscreen: false,
            minimized: false,
        };
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| {
                hits = render_git_dialog(frame, area, &dialog, None, &Theme::current(), 0);
            })
            .unwrap();
        assert_eq!(hits.git_tabs.len(), 3);
        assert_eq!(
            hits.git_tabs
                .iter()
                .map(|(_, page)| *page)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("操作") || rendered.replace(' ', "").contains("操作"));
        assert!(!rendered.replace(' ', "").contains("Git状态"));
        let message = hits.git_message.unwrap();
        let generate = hits.git_generate.unwrap();
        let primary = hits.git_primary.unwrap();
        assert_eq!(hits.hit(message.x, message.y), Some(Hit::GitMessage));
        assert_eq!(hits.hit(generate.x, generate.y), Some(Hit::GitGenerate));
        assert_eq!(hits.hit(primary.x, primary.y), Some(Hit::GitPrimary));
        assert_eq!(primary.x, generate.right() + 1);
        assert_eq!(generate.y, message.y + 1);
        assert_eq!(hits.git_files.len(), 3);
        for (row, index) in &hits.git_files {
            assert_eq!(hits.hit(row.x, row.y), Some(Hit::GitFile(*index)));
        }
        let files = git_changed_files(&dialog.info.raw);
        assert!(files[0].staged);
        assert!(!files[1].staged);
        let mut with_diff = dialog.clone();
        with_diff.diff_path = Some("src/main.rs".into());
        with_diff.diff_text = "@@ -1 +1 @@\n-old code\n+new code".into();
        terminal
            .draw(|frame| {
                hits = render_git_dialog(frame, area, &with_diff, None, &Theme::current(), 0);
            })
            .unwrap();
        let diff = hits.git_diff.expect("selected file has a diff sidebar");
        assert!(hits.git_files.iter().all(|(row, _)| row.right() <= diff.x));
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("+new code"));
        assert!(rendered.contains("-old code"));
        assert!(!files[2].staged);
        assert!(dialog.excluded_files.is_empty());
        assert!(
            hits.git_files
                .iter()
                .all(|(_, index)| !dialog.excluded_files.contains(&files[*index].path))
        );
    }

    #[test]
    fn git_primary_action_respects_selected_files_and_tracking() {
        let mut info = GitStatusInfo {
            branch: "main".into(),
            added: 0,
            removed: 0,
            ahead: 2,
            behind: 1,
            stashes: 0,
            clean: false,
            raw: "## main\n M src/lib.rs\n?? notes.txt".into(),
            diff: String::new(),
            log: String::new(),
        };
        let mut excluded = std::collections::HashSet::new();
        assert_eq!(
            git_primary_action(&info, &excluded),
            GitPrimaryAction::Commit(2)
        );
        excluded.insert("src/lib.rs".into());
        assert_eq!(
            git_primary_action(&info, &excluded),
            GitPrimaryAction::Commit(1)
        );
        excluded.insert("notes.txt".into());
        assert_eq!(
            git_primary_action(&info, &excluded),
            GitPrimaryAction::Commit(0)
        );
        info.raw = "## main".into();
        assert_eq!(git_primary_action(&info, &excluded), GitPrimaryAction::Pull);
        info.behind = 0;
        assert_eq!(git_primary_action(&info, &excluded), GitPrimaryAction::Push);
    }

    #[test]
    fn git_generate_button_uses_running_spinner() {
        let area = Rect::new(0, 0, 110, 30);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        let mut dialog = GitDialog {
            info: GitStatusInfo {
                branch: "main".into(),
                added: 0,
                removed: 0,
                ahead: 0,
                behind: 0,
                stashes: 0,
                clean: false,
                raw: "## main\n M test.rs".into(),
                diff: String::new(),
                log: String::new(),
            },
            page: 0,
            selected: 0,
            message: String::new(),
            editing_message: false,
            generating_message: true,
            syncing: false,
            excluded_files: Default::default(),
            diff_path: None,
            diff_text: String::new(),
            diff_scroll: 0,
            scroll: 0,
            feedback: None,
            fullscreen: false,
            minimized: false,
        };
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| {
                hits = render_git_dialog(frame, area, &dialog, None, &Theme::current(), 1);
            })
            .unwrap();
        let rect = hits.git_generate.unwrap();
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((rect.x + 1, rect.y))
                .unwrap()
                .symbol(),
            spinner(1).to_string()
        );
        dialog.generating_message = false;
        terminal
            .draw(|frame| {
                render_git_dialog(frame, area, &dialog, None, &Theme::current(), 1);
            })
            .unwrap();
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((rect.x + 1, rect.y))
                .unwrap()
                .symbol(),
            "✦"
        );
    }

    #[test]
    fn git_indicator_shows_tracking_counts_for_clean_workspace() {
        let mut status = GitStatusInfo {
            branch: String::new(),
            added: 0,
            removed: 0,
            ahead: 5,
            behind: 2,
            stashes: 2,
            clean: true,
            raw: String::new(),
            diff: String::new(),
            log: String::new(),
        };
        assert_eq!(git_indicator_text(&status), "↑5 ↓2 ⍟2");
        status.clean = false;
        status.added = 3;
        status.removed = 1;
        assert_eq!(git_indicator_text(&status), "+3-1");
    }

    #[test]
    fn git_indicator_stays_left_of_context_with_gap() {
        let inner = Rect::new(2, 0, 76, 1);
        let context = Rect::new(65, 0, 13, 1);
        let git = git_header_rect(inner, context, "↑5 ↓2 ⍟2".width()).unwrap();
        assert_eq!(context.x - git.right(), 2);
        assert!(git_header_rect(Rect::new(2, 0, 15, 1), Rect::new(4, 0, 13, 1), 8).is_none());
    }

    #[test]
    fn untitled_session_has_no_header_title() {
        assert_eq!(title_label(None, None), "");
        assert_eq!(title_label(None, Some("   ")), "");
        assert_eq!(title_label(None, Some("已命名会话")), "已命名会话");
    }

    #[test]
    fn window_title_cycles_star_while_running() {
        assert_eq!(window_title(true, 0, "code", "问候"), "✦ Blora Code · 问候");
        assert_eq!(
            window_title(true, 19, "code", "问候"),
            "✽ Blora Code · 问候"
        );
        assert_eq!(
            window_title(true, 20, "code", "问候"),
            "✼ Blora Code · 问候"
        );
        assert_eq!(window_title(false, 0, "code", "问候"), "Blora Code · 问候");
    }

    #[test]
    fn spinner_ping_pongs_star_sequence() {
        assert_eq!(spinner(0), '✦');
        assert_eq!(spinner(1), '✧');
        assert_eq!(spinner(19), '✽');
        assert_eq!(spinner(20), '✼');
        assert_eq!(spinner(38), '✦');
        assert_eq!(spinner(39), '✧');
    }

    /// Double-width CJK cells leave a placeholder space in the buffer, so
    /// text assertions compare with all spaces removed.
    fn flatten(line: &str) -> String {
        line.chars().filter(|ch| *ch != ' ').collect()
    }

    /// Build a dialog, render it, and return the text lines so tests can
    /// assert content and layout.
    fn render_passport(area: Rect, dialog: &PassportDialog) -> Vec<String> {
        render_passport_with_hits(area, dialog, None).0
    }

    /// Like [`render_passport`], but also returns the hit map so tests can
    /// assert the traffic-light click targets, and takes a hover pointer.
    fn render_passport_with_hits(
        area: Rect,
        dialog: &PassportDialog,
        pointer: Option<(u16, u16)>,
    ) -> (Vec<String>, HitMap) {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| {
                hits = render_passport_dialog(frame, area, dialog, pointer, &Theme::current());
            })
            .unwrap();
        let mut lines = Vec::new();
        for row in 0..area.height {
            let mut line = String::new();
            for col in 0..area.width {
                let cell = terminal
                    .backend()
                    .buffer()
                    .cell((col, row))
                    .cloned()
                    .unwrap_or_default();
                line.push_str(cell.symbol());
            }
            lines.push(line);
        }
        (lines, hits)
    }

    /// Build a provider dialog, render it, and return lines plus hit areas.
    fn render_provider_with_hits(
        area: Rect,
        dialog: &ProviderDialog,
        pointer: Option<(u16, u16)>,
    ) -> (Vec<String>, HitMap) {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| {
                hits = render_provider_dialog(frame, area, dialog, pointer, &Theme::current());
            })
            .unwrap();
        let mut lines = Vec::new();
        for row in 0..area.height {
            let mut line = String::new();
            for col in 0..area.width {
                let cell = terminal
                    .backend()
                    .buffer()
                    .cell((col, row))
                    .cloned()
                    .unwrap_or_default();
                line.push_str(cell.symbol());
            }
            lines.push(line);
        }
        (lines, hits)
    }

    fn sidebar_model<'a>(
        sessions: &'a [SessionSummary],
        sidebar: &'a SidebarState,
    ) -> FrameModel<'a> {
        FrameModel {
            dialog_stack: &[],
            workspace: Path::new("/tmp"),
            sessions,
            index: 0,
            sidebar,
            right_panel: None,
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
            mentions: None,
            search: None,
            hide_tools: false,
            message_roles: true,
            scroll: 0,
            permission_mode: PermissionMode::Ask,
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
            running: false,
            tick: 0,
            pointer: None,
            text_selection: None,
        }
    }

    fn sidebar_frame(
        width: u16,
        height: u16,
        model: &FrameModel<'_>,
    ) -> (Terminal<TestBackend>, HitMap) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| {
                hits = draw(frame, model, &mut TranscriptCache::default());
            })
            .unwrap();
        (terminal, hits)
    }

    #[test]
    fn sidebar_thresholds_empty_lists_and_canvas_offsets() {
        let state = SidebarState::default();
        let mut model = sidebar_model(&[], &state);
        model.input = "draft 中文";
        for (width, height, visible) in [
            (99, 30, false),
            (100, 17, false),
            (100, 18, true),
            (120, 30, true),
            (140, 10, false),
            (40, 24, false),
            (24, 10, false),
        ] {
            let (terminal, hits) = sidebar_frame(width, height, &model);
            assert_eq!(hits.sidebar.is_some(), visible, "{width}×{height}");
            assert_eq!(
                hits.composer.x,
                inset(Rect::new(
                    if visible { 28 } else { 0 },
                    0,
                    if visible { width - 28 } else { width },
                    2
                ))
                .x
            );
            assert_eq!(hits.transcript.x, hits.composer.x);
            if visible {
                assert_eq!(hits.sidebar.unwrap().width, 28);
                let buffer = terminal.backend().buffer();
                assert_eq!(buffer[(3, 1)].bg, Theme::current().bg_raised);
                let projects = hits.sidebar_projects.unwrap();
                let recent = hits.sidebar_sessions.unwrap();
                assert_eq!(recent.y, projects.bottom() + 2);
                assert!(frame_text(&terminal).contains("未登录"));
                assert!(hits.sidebar_rows.iter().all(|(rect, _)| rect.right() < 28));
                assert!(frame_text(&terminal).contains("暂无会话"));
                assert_eq!(hits.hit(2, 3), Some(Hit::Hint(HintAction::New)));
                assert_eq!(hits.hit(2, 4), Some(Hit::SessionPicker));
                assert_eq!(hits.hit(2, 6), Some(Hit::ProjectPicker));
                assert_eq!(hits.hit(2, height - 2), Some(Hit::SidebarSettings));
                assert_eq!(hits.hit(2, 8), None);
            }
        }
        assert_eq!(model.input, "draft 中文");
        let (sidebar, canvas) = canvas_regions(Rect::new(5, 7, 100, 18));
        assert_eq!(sidebar, Some(Rect::new(5, 7, 28, 18)));
        assert_eq!(canvas, Rect::new(33, 7, 72, 18));
    }

    #[test]
    fn sidebar_scroll_hits_filtering_and_current_visibility() {
        let mut sessions = picker_sessions(30);
        sessions[0].workspace_path = "/tmp/other".into();
        sessions[29].title = Some("很长的中文会话标题".repeat(10));
        sessions[28].title = Some("tui".into());
        let workspace = Path::new("/tmp/中文项目");
        let mut state = SidebarState::default();
        state.sync(&sessions, 29, workspace, 18);
        let mut model = sidebar_model(&sessions, &state);
        model.index = 29;
        let (terminal, hits) = sidebar_frame(100, 18, &model);
        assert_eq!(hits.sidebar_project_max, 0);
        assert_eq!(hits.sidebar_session_max, 24);
        assert!(
            hits.sidebar_rows
                .iter()
                .any(|(_, hit)| *hit == Hit::SidebarSession(29))
        );
        assert!(
            !hits
                .sidebar_rows
                .iter()
                .any(|(_, hit)| *hit == Hit::SidebarSession(0))
        );
        let (rect, _) = hits
            .sidebar_rows
            .iter()
            .find(|(_, hit)| *hit == Hit::SidebarSession(29))
            .unwrap();
        assert_eq!(hits.hit(rect.x, rect.y), Some(Hit::SidebarSession(29)));
        assert!(frame_text(&terminal).contains("很长的中文会话标题"));
        assert!(frame_text(&terminal).contains("…"));
        assert!(frame_text(&terminal).contains("未命名会话"));
        assert_eq!(crate::project_paths(&sessions).len(), 2);
        let list = hits.sidebar_sessions.unwrap();
        assert!(state.scroll(&hits, list.x + 1, list.y, true));
        let manual = state.sessions;
        state.sync(&sessions, 29, workspace, 18);
        assert_eq!(state.sessions, manual);
        let mut model = sidebar_model(&sessions, &state);
        model.index = 29;
        let (_, scrolled) = sidebar_frame(100, 18, &model);
        assert_eq!(
            scrolled.hit(2, list.y),
            Some(Hit::SidebarSession(manual + 1))
        );
        for _ in 0..100 {
            state.scroll(&hits, list.x + 1, list.y, true);
        }
        assert_eq!(state.sessions, 0);
        for _ in 0..100 {
            state.scroll(&hits, list.x + 1, list.y, false);
        }
        assert_eq!(state.sessions, hits.sidebar_session_max);
        assert!(!state.scroll(&hits, 50, list.y, true));
        state.sync(&sessions, 1, workspace, 18);
        assert_eq!(state.sessions, 0);
        state.sync(&sessions, 29, workspace, 30);
        state.sync(&sessions, 29, workspace, 18);
        assert_eq!(state.sessions, 24);
    }

    #[test]
    fn sidebar_project_scroll_is_independent_and_clamped() {
        let mut sessions = picker_sessions(20);
        for (index, session) in sessions.iter_mut().enumerate() {
            session.workspace_path = format!("/tmp/project-{index:02}");
        }
        let mut state = SidebarState::default();
        state.sync(&sessions, 0, Path::new("/tmp"), 18);
        let (_, hits) = sidebar_frame(100, 18, &sidebar_model(&sessions, &state));
        let projects = hits.sidebar_projects.unwrap();
        let recent = hits.sidebar_sessions.unwrap();
        assert!(state.scroll(&hits, 2, projects.y, false));
        assert_eq!(state.projects, 3);
        assert_eq!(state.sessions, 0);
        let (_, scrolled) = sidebar_frame(100, 18, &sidebar_model(&sessions, &state));
        assert_eq!(scrolled.hit(2, projects.y), Some(Hit::ProjectPickerRow(3)));
        assert!(state.scroll(&hits, 2, recent.y, false));
        assert_eq!(state.projects, 3);
        assert_eq!(state.sessions, 0);
        for _ in 0..100 {
            state.scroll(&hits, 2, projects.y, false);
        }
        assert_eq!(state.projects, 18);
        state.sync(&sessions, 0, Path::new("/tmp"), 18);
        assert_eq!(state.projects, 18);
        state.sync(&sessions, 1, Path::new("/tmp"), 18);
        assert_eq!(state.projects, 1);
        assert!(state.scroll(&hits, 2, 1, false));
        assert_eq!(state.projects, 1);
        sessions.clear();
        state.sync(&sessions, 0, Path::new("/tmp"), 18);
        assert_eq!((state.projects, state.sessions), (0, 0));
    }

    #[test]
    fn sidebar_modal_uses_full_terminal_and_blocks_background() {
        let state = SidebarState::default();
        let mut model = sidebar_model(&[], &state);
        let dialog = SettingsDialog {
            page: 0,
            selected: 0,
            fullscreen: false,
            minimized: false,
        };
        model.settings_dialog = Some(&dialog);
        model.dialog_stack = &[DialogId::Settings];
        let (modal, hits) = sidebar_frame(120, 30, &model);
        assert!(hits.modal);
        assert_eq!(hits.hit(2, 3), None);
        assert!(hits.sidebar.is_none());
        let rect = hits.modal_area.unwrap();
        assert!((i32::from(rect.x) * 2 + i32::from(rect.width) - 120).abs() <= 2);
        model.dialog_stack = &[];
        let (base, base_hits) = sidebar_frame(120, 30, &model);
        assert_ne!(
            modal.backend().buffer()[(2, 3)].fg,
            base.backend().buffer()[(2, 3)].fg
        );
        let mut state = SidebarState::default();
        assert!(!state.scroll(&hits, 2, 10, false));
        let mut blocked = base_hits;
        blocked.modal = true;
        blocked.modal_area = Some(rect);
        assert_eq!(blocked.hit(2, 3), None);
        assert!(!state.scroll(&blocked, 2, 10, false));
    }

    fn picker_sessions(count: usize) -> Vec<SessionSummary> {
        (0..count)
            .map(|index| SessionSummary {
                id: blora_types::SessionId::generate(),
                title: Some(format!("中文会话 {index}")),
                workspace_path: "/tmp/中文项目".to_owned(),
                mode: Mode::Code,
                status: blora_types::SessionStatus::Active,
                updated_at: chrono::Utc::now(),
                last_sequence: 1,
            })
            .collect()
    }

    fn render_picker_with_hits(
        area: Rect,
        picker: &SessionPicker,
        sessions: &[SessionSummary],
    ) -> (Terminal<TestBackend>, HitMap) {
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| {
                hits =
                    render_session_picker(frame, area, picker, sessions, None, &Theme::current());
            })
            .unwrap();
        (terminal, hits)
    }

    #[test]
    fn session_picker_layout_keeps_selected_rows_visible_on_narrow_screens() {
        let sessions = picker_sessions(30);
        for (width, height) in [(140, 40), (80, 24), (24, 10), (20, 5)] {
            let mut picker = SessionPicker::new(&sessions, None, 0);
            for selected in [0, 15, 29, 0] {
                picker.selected = selected;
                let (_, hits) =
                    render_picker_with_hits(Rect::new(0, 0, width, height), &picker, &sessions);
                let outer = hits.modal_area.unwrap();
                let search = hits.session_search.unwrap();
                assert_eq!(search.height, 1);
                assert_eq!(hits.hit(search.x, search.y), Some(Hit::SessionSearch));
                assert!(
                    hits.session_picker_rows
                        .iter()
                        .any(|(_, index)| *index == selected)
                );
                for (rect, index) in &hits.session_picker_rows {
                    assert!(rect.right() < outer.right());
                    assert!(rect.bottom() < outer.bottom());
                    assert_eq!(
                        hits.hit(rect.x, rect.y),
                        Some(Hit::SessionPickerRow(*index))
                    );
                }
                assert_eq!(hits.hit(0, 0), None);
                if width == 140 {
                    assert_eq!(outer.width, 110);
                }
            }
        }
    }

    #[test]
    fn session_picker_content_rows_use_picker_indices_and_highlight_matches() {
        let sessions = picker_sessions(2);
        let mut picker = SessionPicker::new(&sessions, None, 0);
        picker.query = "命中".to_owned();
        picker.rows = vec![
            crate::session_picker::PickerRow {
                id: sessions[1].id.clone(),
                content: None,
            },
            crate::session_picker::PickerRow {
                id: sessions[0].id.clone(),
                content: Some("这里命中正文".to_owned()),
            },
        ];
        picker
            .previews
            .insert(sessions[1].id.to_string(), (1, "缓存预览".to_owned()));
        picker.selected = 1;
        picker.searching = true;
        let (terminal, hits) =
            render_picker_with_hits(Rect::new(0, 0, 120, 24), &picker, &sessions);
        let text = frame_text(&terminal);
        assert!(text.contains("正在深入搜索会话内容"), "{text}");
        assert!(text.contains("缓存预览"), "{text}");
        assert!(text.contains("这里命中正文"), "{text}");
        assert_eq!(
            hits.session_picker_rows
                .iter()
                .map(|(_, index)| *index)
                .collect::<Vec<_>>(),
            vec![0, 0, 1, 1]
        );
        let preview = hits.session_picker_rows[3].0;
        assert_eq!(
            terminal.backend().buffer()[(preview.x + 6, preview.y)].fg,
            Theme::current().amber
        );
        picker.searching = false;
        let (terminal, _) = render_picker_with_hits(Rect::new(0, 0, 120, 24), &picker, &sessions);
        assert!(frame_text(&terminal).contains("1 条结果"));
        picker.error = Some("测试错误".to_owned());
        let (terminal, _) = render_picker_with_hits(Rect::new(0, 0, 120, 24), &picker, &sessions);
        assert!(frame_text(&terminal).contains("内容搜索失败：测试错误"));
    }

    #[test]
    fn session_picker_empty_fullscreen_and_tiny_layouts_are_safe() {
        let picker = SessionPicker::new(&[], None, 0);
        let (terminal, hits) = render_picker_with_hits(Rect::new(0, 0, 80, 24), &picker, &[]);
        assert!(frame_text(&terminal).contains("没有匹配的会话"));
        assert!(hits.session_picker_rows.is_empty());
        let sessions = picker_sessions(1);
        let mut picker = SessionPicker::new(&sessions, None, 0);
        picker.fullscreen = true;
        let (_, hits) = render_picker_with_hits(Rect::new(0, 0, 80, 24), &picker, &sessions);
        assert_eq!(hits.modal_area.unwrap().width, 78);
        assert_eq!(hits.session_picker_rows.len(), 2);
        for (width, height) in [(19, 10), (20, 3), (20, 4)] {
            let (_, hits) =
                render_picker_with_hits(Rect::new(0, 0, width, height), &picker, &sessions);
            assert!(hits.session_picker_rows.is_empty());
        }
    }

    #[test]
    fn session_picker_minimized_has_only_window_controls() {
        let sessions = picker_sessions(1);
        let mut picker = SessionPicker::new(&sessions, None, 0);
        picker.minimized = true;
        let (terminal, hits) = render_picker_with_hits(Rect::new(0, 0, 80, 24), &picker, &sessions);
        assert!(hits.session_search.is_none());
        assert!(hits.session_picker_rows.is_empty());
        assert!(!frame_text(&terminal).contains("中文会话"));
        assert!(hits.traffic_lights.iter().all(Option::is_some));
    }

    #[test]
    fn shared_dialog_frame_keeps_controls_inside_the_border() {
        for (width, height, minimized) in [(80, 24, false), (24, 5, true), (20, 3, true)] {
            let area = Rect::new(0, 0, width, height);
            let outer = dialog_outer(area, 72, 17, false, minimized).unwrap();
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            let mut hits = HitMap::default();
            let mut inner = Rect::default();
            terminal
                .draw(|frame| {
                    inner = paint_dialog_frame(
                        frame,
                        outer,
                        "测试对话框",
                        None,
                        &Theme::current(),
                        &mut hits,
                    );
                })
                .unwrap();
            assert!(outer.contains(ratatui::layout::Position::new(inner.x, inner.y)));
            for (index, expected) in [
                Hit::TrafficClose,
                Hit::TrafficMinimize,
                Hit::TrafficOpenBrowser,
            ]
            .into_iter()
            .enumerate()
            {
                let rect = hits.traffic_lights[index].unwrap();
                assert!(inner.contains(ratatui::layout::Position::new(rect.x, rect.y)));
                assert_eq!(hits.hit(rect.x, rect.y), Some(expected));
            }
        }
    }

    #[test]
    fn passport_dialog_is_centered_and_complete() {
        let dialog = PassportDialog {
            user_code: "ABCD-2345".to_owned(),
            verification_uri: "https://passport.bloret.net/oauth/device?user_code=ABCD-2345"
                .to_owned(),
            opened_browser: false,
        };
        let area = Rect::new(0, 0, 90, 30);
        let (lines, hits) = render_passport_with_hits(area, &dialog, None);
        let title_row = lines
            .iter()
            .position(|line| flatten(line).contains("BloretPassPort登录"))
            .expect("dialog title rendered");
        // The title row leads with a padding space, then the three dots
        // separated by single spaces. Offsets are relative to the dialog's
        // inner area (the whole-screen row also contains the left border).
        let inner_x = hits.traffic_lights[0]
            .expect("red hit area")
            .x
            .saturating_sub(1);
        let title_chars: Vec<char> = lines[title_row].chars().collect();
        let red_col = title_chars
            .iter()
            .position(|ch| *ch == '●')
            .expect("red dot glyph");
        assert_eq!(
            u16::try_from(red_col).unwrap_or(0),
            inner_x + 1,
            "one space inside the frame before the red dot"
        );
        assert_eq!(title_chars[red_col + 1], ' ');
        assert_eq!(title_chars[red_col + 2], '●');
        assert_eq!(title_chars[red_col + 3], ' ');
        assert_eq!(title_chars[red_col + 4], '●');
        let code_row = lines
            .iter()
            .position(|line| flatten(line).contains("ABCD-2345"))
            .expect("device code rendered");
        let link_row = lines
            .iter()
            .position(|line| flatten(line).contains("passport.bloret.net"))
            .expect("verification uri rendered");
        let hint_row = lines
            .iter()
            .position(|line| {
                let flat = flatten(line);
                flat.contains("隐藏对话框") && flat.contains("等待授权")
            })
            .expect("footer hint rendered");
        assert!(code_row > title_row);
        assert!(link_row > code_row);
        assert!(hint_row > link_row);
        // All three dots expose click targets, one column apart, on the title
        // row after the leading space.
        let dot_column = |index: usize| {
            hits.traffic_lights[index]
                .expect("traffic light hit area")
                .x
        };
        assert_eq!(dot_column(0), u16::try_from(red_col).unwrap_or(0));
        assert_eq!(dot_column(1), dot_column(0) + 2);
        assert_eq!(dot_column(2), dot_column(0) + 4);
        assert_eq!(
            hits.traffic_lights[0].unwrap().y,
            u16::try_from(title_row).unwrap_or(0)
        );
        // Centered horizontally: the top border's corner margins are symmetric.
        let frame_row = &lines[title_row - 1];
        // `find` returns a byte offset; these box glyphs are multi-byte UTF-8.
        let column = |needle: char| {
            frame_row
                .char_indices()
                .position(|(_, ch)| ch == needle)
                .expect("border corner")
        };
        let left = column('╭');
        let right = column('╮');
        assert_eq!(left, area.width as usize - right - 1);
    }

    #[test]
    fn traffic_lights_show_symbols_on_hover() {
        let dialog = PassportDialog {
            user_code: "ABCD-2345".to_owned(),
            verification_uri: "https://passport.bloret.net".to_owned(),
            opened_browser: false,
        };
        let area = Rect::new(0, 0, 70, 20);
        let (lines, hits) = render_passport_with_hits(area, &dialog, None);
        let red = hits.traffic_lights[0].expect("red hit area");
        let row_chars = |line: &str| -> Vec<char> { line.chars().collect() };
        // No hover: plain dots.
        assert_eq!(row_chars(&lines[red.y as usize])[red.x as usize], '●');
        // Hovering the red dot swaps its glyph for the close symbol.
        let (lines, _) = render_passport_with_hits(area, &dialog, Some((red.x, red.y)));
        let chars = row_chars(&lines[red.y as usize]);
        assert_eq!(chars[red.x as usize], '✕', "red shows ✕ on hover");
        assert_eq!(chars[red.x as usize + 2], '●', "yellow stays a dot");
        assert_eq!(chars[red.x as usize + 4], '●', "green stays a dot");
        // Hovering the green dot shows the fullscreen plus.
        let green = hits.traffic_lights[2].expect("green hit area");
        let (lines, _) = render_passport_with_hits(area, &dialog, Some((green.x, green.y)));
        let chars = row_chars(&lines[green.y as usize]);
        assert_eq!(chars[green.x as usize], '+', "green shows + on hover");
    }

    #[test]
    fn imagine_transcript_lists_saved_images_and_failures() {
        let mut projection = SessionProjection::new();
        projection.session = Some(blora_session::SessionRecord {
            id: blora_types::SessionId::generate(),
            title: None,
            workspace_path: "/tmp".to_owned(),
            mode: Mode::Imagine,
            status: blora_types::SessionStatus::Active,
            parent_session_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        });
        let hash = "a".repeat(64);
        projection.transcript.push(TranscriptItem::User {
            text: "一张珊瑚壁纸\n画幅 16:9，张数 1".to_owned(),
            event_id: blora_types::EventId::generate(),
        });
        projection.transcript.push(TranscriptItem::Tool {
            name: "generate_image".to_owned(),
            status: "completed".to_owned(),
            event_id: blora_types::EventId::generate(),
            arguments: None,
            output: Some(format!(".blora/images/sess/{hash}.png image/png 1x1")),
            call_id: None,
        });
        projection.transcript.push(TranscriptItem::Tool {
            name: "generate_image".to_owned(),
            status: "failed".to_owned(),
            event_id: blora_types::EventId::generate(),
            arguments: None,
            output: Some("image API HTTP 401".to_owned()),
            call_id: None,
        });
        let rendered =
            transcript_lines(&projection, None, false, true, 78, "you", &Theme::current());
        let text = rendered
            .lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("已生成"));
        assert!(text.contains("一张珊瑚壁纸"));
        assert!(text.contains(".blora/images/sess/"));
        assert!(text.contains("HTTP 401"));
        assert!(!text.contains("you"));
    }

    #[test]
    fn shared_conversation_matches_main_for_roles_tools_streaming_and_scroll() {
        let mut projection = SessionProjection::new();
        projection.transcript.push(TranscriptItem::User {
            text: "用户 **Markdown**".into(),
            event_id: blora_types::EventId::generate(),
        });
        projection.transcript.push(TranscriptItem::Tool {
            name: "read_file".into(),
            status: "completed".into(),
            event_id: blora_types::EventId::generate(),
            arguments: None,
            output: None,
            call_id: None,
        });
        let event = blora_events::EventEnvelope::stamped(
            blora_events::NewEvent::new(
                blora_types::SessionId::generate(),
                blora_events::KnownPayload::AssistantDelta(blora_events::AssistantDelta {
                    text: "流式 **正文**".into(),
                }),
            ),
            1,
        )
        .unwrap();
        projection.apply(&event).unwrap();
        let sidebar = SidebarState::default();
        let mut model = sidebar_model(&[], &sidebar);
        model.projection = Some(&projection);
        model.running = true;
        for roles in [false, true] {
            for hide in [false, true] {
                for scroll in [0, 2, 100] {
                    model.message_roles = roles;
                    model.hide_tools = hide;
                    model.scroll = scroll;
                    let mut main = Terminal::new(TestBackend::new(36, 8)).unwrap();
                    let mut side = Terminal::new(TestBackend::new(36, 8)).unwrap();
                    let mut hits = HitMap::default();
                    let mut rows = Vec::new();
                    main.draw(|frame| {
                        render_transcript(
                            frame,
                            Rect::new(0, 0, 36, 8),
                            &model,
                            &Theme::current(),
                            &mut hits,
                            &mut TranscriptCache::default(),
                        )
                    })
                    .unwrap();
                    side.draw(|frame| {
                        rows = crate::conversation::render_transcript(
                            frame,
                            inset(Rect::new(0, 0, 36, 8)),
                            &model,
                            &Theme::current(),
                            &mut TranscriptCache::default(),
                        )
                    })
                    .unwrap();
                    assert_eq!(main.backend().buffer(), side.backend().buffer());
                    assert_eq!(hits.tool_summary_rows, rows);
                    if hide {
                        assert!(rows.is_empty());
                    }
                }
            }
        }
    }

    #[test]
    fn side_tool_summary_hit_keeps_session_owner() {
        let owner = blora_types::SessionId::generate();
        let mut hits = HitMap::default();
        hits.tool_summary_rows = vec![(Rect::new(1, 1, 10, 1), vec![0])];
        hits.side_tool_summary_rows = vec![(Rect::new(40, 1, 10, 1), owner.clone(), vec![0])];
        assert_eq!(hits.hit(1, 1), Some(Hit::ToolSummary(vec![0])));
        assert_eq!(hits.hit(40, 1), Some(Hit::SideToolSummary(owner, vec![0])));
        assert_eq!(hits.hit(50, 1), None);
    }

    #[test]
    fn message_role_toggle_updates_cached_history_and_streaming() {
        let mut projection = SessionProjection::new();
        projection.transcript.push(TranscriptItem::User {
            text: "用户正文".to_owned(),
            event_id: blora_types::EventId::generate(),
        });
        projection.transcript.push(TranscriptItem::Assistant {
            text: "**助手正文**".to_owned(),
            event_id: blora_types::EventId::generate(),
        });
        let event = blora_events::EventEnvelope::stamped(
            blora_events::NewEvent::new(
                blora_types::SessionId::generate(),
                blora_events::KnownPayload::AssistantDelta(blora_events::AssistantDelta {
                    text: "流式正文".to_owned(),
                }),
            ),
            1,
        )
        .unwrap();
        projection.apply(&event).unwrap();
        for width in [20, 80] {
            let theme = Theme::current();
            let mut cache = TranscriptCache::default();
            cache.ensure(
                &projection,
                "session",
                None,
                false,
                true,
                width,
                "you",
                &theme,
            );
            assert!(
                cache
                    .lines
                    .iter()
                    .any(|line| line.to_string().contains("┃ you"))
            );
            cache.ensure(
                &projection,
                "session",
                None,
                false,
                false,
                width,
                "you",
                &theme,
            );
            assert_eq!(cache.rebuilds(), 2);
            let text = cache
                .lines
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            for body in ["用户正文", "助手正文", "流式正文"] {
                assert!(text.contains(body), "{text}");
            }
            for label in ["┃", "you", "Blora", "blora"] {
                assert!(!text.contains(label), "{text}");
            }
            assert!(cache.live_visible);
            cache.ensure(
                &projection,
                "session",
                None,
                false,
                true,
                width,
                "you",
                &theme,
            );
            assert_eq!(cache.rebuilds(), 3);
            assert!(
                cache
                    .lines
                    .iter()
                    .any(|line| line.to_string().contains("┃ Blora"))
            );
        }
    }

    #[test]
    fn tool_summary_click_target_covers_only_the_visible_text() {
        let mut projection = SessionProjection::new();
        for name in ["search", "read_file"] {
            projection.transcript.push(TranscriptItem::Tool {
                name: name.to_owned(),
                status: "completed".to_owned(),
                event_id: blora_types::EventId::generate(),
                arguments: None,
                output: None,
                call_id: None,
            });
        }
        let area = Rect::new(4, 5, 80, 8);
        let rendered =
            transcript_lines(&projection, None, false, true, 78, "you", &Theme::current());
        let rows = tool_summary_hit_rows(&rendered.marks, rendered.lines.len(), area, 0);
        assert_eq!(rows.len(), 1);
        let (target, indices) = &rows[0];
        assert_eq!(indices, &vec![0, 1]);
        assert_eq!(target.x, area.x + 2);
        assert_eq!(
            target.width as usize,
            blora_session::summarize_tool_run(
                &projection.transcript.iter().collect::<Vec<_>>(),
                false
            )
            .width()
        );
        assert!(target.width < area.width - 2);
        let mut hits = HitMap::default();
        hits.tool_summary_rows = rows.clone();
        assert_eq!(
            hits.hit(target.x, target.y),
            Some(Hit::ToolSummary(vec![0, 1]))
        );
        assert_ne!(
            hits.hit(area.x, target.y),
            Some(Hit::ToolSummary(vec![0, 1]))
        );
        assert_ne!(
            hits.hit(target.right(), target.y),
            Some(Hit::ToolSummary(vec![0, 1]))
        );
        let scrolled = rendered.lines.len() + 12;
        assert!(tool_summary_hit_rows(&rendered.marks, scrolled, area, 0).is_empty());
    }

    #[test]
    fn transcript_cache_keeps_layout_until_content_inputs_change() {
        let mut projection = SessionProjection::new();
        projection.transcript.push(TranscriptItem::User {
            text: "hello".to_owned(),
            event_id: blora_types::EventId::generate(),
        });
        let theme = Theme::current();
        let mut cache = TranscriptCache::default();
        cache.ensure(&projection, "ses_a", None, false, true, 40, "you", &theme);
        assert_eq!(cache.rebuilds(), 1);
        cache.ensure(&projection, "ses_a", None, false, true, 40, "you", &theme);
        assert_eq!(cache.rebuilds(), 1);
        cache.ensure(&projection, "ses_a", None, false, true, 80, "you", &theme);
        assert_eq!(cache.rebuilds(), 2);

        let session = blora_types::SessionId::generate();
        let event = blora_events::EventEnvelope::stamped(
            blora_events::NewEvent::new(
                session,
                blora_events::KnownPayload::AssistantDelta(blora_events::AssistantDelta {
                    text: "Hi".to_owned(),
                }),
            ),
            1,
        )
        .unwrap();
        projection.apply(&event).unwrap();
        cache.ensure(&projection, "ses_a", None, false, true, 80, "you", &theme);
        assert_eq!(cache.rebuilds(), 3);
        assert!(cache.live_visible);
        cache.ensure(&projection, "ses_a", None, false, true, 80, "you", &theme);
        assert_eq!(cache.rebuilds(), 3);
    }

    #[test]
    fn tool_dialog_controls_take_priority_over_background_tool_rows() {
        let area = Rect::new(0, 0, 80, 24);
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        let dialog = ToolDialog {
            tools: Vec::new(),
            scroll: 0,
            fullscreen: false,
            minimized: false,
        };
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| {
                hits = render_tool_dialog(frame, area, &dialog, None, &Theme::current());
            })
            .unwrap();
        for (index, expected) in [
            Hit::TrafficClose,
            Hit::TrafficMinimize,
            Hit::TrafficOpenBrowser,
        ]
        .into_iter()
        .enumerate()
        {
            let rect = hits.traffic_lights[index].expect("window control hit area");
            hits.tool_summary_rows.push((rect, vec![0]));
            assert_eq!(hits.hit(rect.x, rect.y), Some(expected));
        }
    }

    #[test]
    fn traffic_lights_resolve_to_hits() {
        let dialog = PassportDialog {
            user_code: "ABCD-2345".to_owned(),
            verification_uri: "https://passport.bloret.net".to_owned(),
            opened_browser: false,
        };
        let area = Rect::new(0, 0, 70, 20);
        let (_, hits) = render_passport_with_hits(area, &dialog, None);
        let cases: [(usize, Hit); 3] = [
            (0, Hit::TrafficClose),
            (1, Hit::TrafficMinimize),
            (2, Hit::TrafficOpenBrowser),
        ];
        for (index, expected) in cases {
            let rect = hits.traffic_lights[index].expect("hit area");
            assert_eq!(
                hits.hit(rect.x, rect.y),
                Some(expected),
                "dot {index} resolves"
            );
        }
    }

    #[test]
    fn passport_dialog_survives_narrow_screens() {
        let dialog = PassportDialog {
            user_code: "ABCD-2345".to_owned(),
            verification_uri: "https://passport.bloret.net".to_owned(),
            opened_browser: true,
        };
        // Too small: nothing drawn, no panic.
        let lines = render_passport(Rect::new(0, 0, 16, 4), &dialog);
        assert!(lines.iter().all(|line| line.trim().is_empty()));
        // Small but sufficient: the browser-opened status replaces 等待授权.
        let lines = render_passport(Rect::new(0, 0, 70, 20), &dialog);
        assert!(
            lines
                .iter()
                .any(|line| flatten(line).contains("已打开浏览器"))
        );
    }

    #[test]
    fn splits_dialog_rows_evenly_with_remainder_on_top() {
        let inner = Rect::new(3, 2, 40, 7);
        let rows = split_dialog_rows(inner, 5);
        let total: u16 = rows.iter().map(|row| row.height).sum();
        assert_eq!(total, 7);
        // 7 rows over 5 slots: the first two get the extra row.
        assert_eq!(rows[0].height, 2);
        assert_eq!(rows[1].height, 2);
        assert!(rows[2..].iter().all(|row| row.height == 1));
        for (index, row) in rows.iter().enumerate() {
            let expected_y = inner.y + rows[..index].iter().map(|row| row.height).sum::<u16>();
            assert_eq!(row.y, expected_y);
            assert_eq!(row.x, inner.x);
            assert_eq!(row.width, inner.width);
        }
    }

    fn sample_provider_dialog() -> ProviderDialog {
        ProviderDialog {
            options: vec![
                ProviderOption {
                    id: "blora".to_owned(),
                    display: "Bloret PassPort".to_owned(),
                    hint: "默认 · 200 次/天".to_owned(),
                    available: true,
                    models: vec![ModelOption {
                        id: "blora".to_owned(),
                        display: "Blora".to_owned(),
                        hint: String::new(),
                    }],
                },
                ProviderOption {
                    id: "openai".to_owned(),
                    display: "OpenAI".to_owned(),
                    hint: "已保存".to_owned(),
                    available: false,
                    models: vec![ModelOption {
                        id: "gpt-4o-mini".to_owned(),
                        display: "gpt-4o-mini".to_owned(),
                        hint: String::new(),
                    }],
                },
                ProviderOption {
                    id: blora_catalog::ADD_PROVIDER_ID.to_owned(),
                    display: "+ 添加供应商".to_owned(),
                    hint: "从 models.dev 接入".to_owned(),
                    available: true,
                    models: Vec::new(),
                },
            ],
            selected: 0,
            pane: ProviderPane::Providers,
            model_selected: 0,
            fullscreen: false,
            minimized: false,
        }
    }

    #[test]
    fn provider_dialog_renders_rows_and_marks_selection() {
        let dialog = sample_provider_dialog();
        let area = Rect::new(0, 0, 80, 24);
        let (lines, hits) = render_provider_with_hits(area, &dialog, None);
        assert_eq!(hits.provider_rows.len(), 3, "one hit area per row");
        assert!(!hits.provider_model_rows.is_empty());
        let find_row = |needle: &str| {
            lines
                .iter()
                .position(|line| flatten(line).contains(needle))
                .unwrap_or_else(|| panic!("{needle} rendered"))
        };
        let title_row = find_row("选择供应商与模型");
        let blora_row = find_row("BloretPassPort");
        let openai_row = find_row("OpenAI");
        let add_row = find_row("+添加供应商");
        assert!(blora_row > title_row);
        assert!(openai_row > blora_row);
        assert!(add_row > openai_row);
        let marker_row = &lines[blora_row];
        assert!(
            marker_row.contains('❯'),
            "selected row has ❯, got {marker_row}"
        );
        assert!(
            lines.iter().all(|line| !flatten(line).contains("默认（")),
            "no synthetic default row"
        );
        let unavailable_row = &lines[openai_row + 1];
        assert!(
            flatten(unavailable_row).contains("不可用"),
            "unavailable provider is labeled, got {unavailable_row}"
        );
        for (rect, index) in &hits.provider_rows {
            assert_eq!(hits.hit(rect.x, rect.y), Some(Hit::ProviderRow(*index)));
        }
    }

    #[test]
    fn provider_headings_and_rows_stay_below_title_bar() {
        let dialog = sample_provider_dialog();
        for (width, height) in [(100, 30), (80, 24), (60, 16), (40, 24), (24, 10)] {
            let (lines, hits) =
                render_provider_with_hits(Rect::new(0, 0, width, height), &dialog, None);
            let title_y = hits.traffic_lights[0].unwrap().y;
            for (rect, index) in &hits.provider_rows {
                assert!(rect.y > title_y);
                assert_eq!(hits.hit(rect.x, rect.y), Some(Hit::ProviderRow(*index)));
            }
            for (rect, index) in &hits.provider_model_rows {
                assert!(rect.y > title_y);
                assert_eq!(
                    hits.hit(rect.x, rect.y),
                    Some(Hit::ProviderModelRow(*index))
                );
            }
            let title = flatten(&lines[usize::from(title_y)]);
            assert!(
                !title.contains("供应商选择"),
                "heading overlaps title: {title}"
            );
            assert!(
                !title.ends_with("模型模型"),
                "model heading overlaps title: {title}"
            );
            if width >= 80 {
                assert!(title.contains("选择供应商与模型·Blora原生设置"));
                let headings = flatten(&lines[usize::from(title_y + 1)]);
                assert!(headings.contains("供应商"));
                assert!(headings.contains("模型"));
            }
        }
    }

    #[test]
    fn provider_dialog_keeps_long_list_selection_visible() {
        let mut dialog = sample_provider_dialog();
        dialog.options[0].models = (0..30)
            .map(|index| ModelOption {
                id: format!("model-{index}"),
                display: format!("model-{index}"),
                hint: "很长的模型说明".repeat(20),
            })
            .collect();
        dialog.pane = ProviderPane::Models;
        dialog.model_selected = 29;
        for (width, height) in [(100, 30), (60, 16), (40, 12)] {
            let (lines, hits) =
                render_provider_with_hits(Rect::new(0, 0, width, height), &dialog, None);
            assert!(lines.iter().any(|line| flatten(line).contains("model-29")));
            let (rect, index) = hits.provider_model_rows.last().unwrap();
            assert_eq!(*index, 29);
            assert_eq!(hits.hit(rect.x, rect.y), Some(Hit::ProviderModelRow(29)));
            assert!(lines.iter().any(|line| line.contains('…')));
        }
    }

    #[test]
    fn provider_dialog_hover_highlights_row() {
        let dialog = sample_provider_dialog();
        let area = Rect::new(0, 0, 80, 24);
        let (_, hits) = render_provider_with_hits(area, &dialog, None);
        let third = hits.provider_rows[1].0;
        let (lines, _) = render_provider_with_hits(area, &dialog, Some((third.x + 1, third.y)));
        let openai_row = lines
            .iter()
            .position(|line| flatten(line).contains("OpenAI"))
            .expect("openai row");
        // Hovering moves the ❯ marker to the hovered row.
        let marker_row = &lines[openai_row];
        assert!(
            marker_row.contains('❯'),
            "hovered row gets ❯, got {marker_row}"
        );
    }

    fn render_add_provider_with_hits(
        area: Rect,
        dialog: &AddProviderDialog,
        pointer: Option<(u16, u16)>,
    ) -> (Vec<String>, HitMap) {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| {
                hits = render_add_provider_dialog(frame, area, dialog, pointer, &Theme::current());
            })
            .unwrap();
        let mut lines = Vec::new();
        for row in 0..area.height {
            let mut line = String::new();
            for col in 0..area.width {
                let cell = terminal
                    .backend()
                    .buffer()
                    .cell((col, row))
                    .cloned()
                    .unwrap_or_default();
                line.push_str(cell.symbol());
            }
            lines.push(line);
        }
        (lines, hits)
    }

    #[test]
    fn add_provider_lists_custom_first() {
        let dialog = AddProviderDialog {
            step: AddProviderStep::Catalog,
            catalog: vec![blora_catalog::CatalogEntry {
                id: "openai".to_owned(),
                name: "OpenAI".to_owned(),
                api: Some("https://api.openai.com/v1".to_owned()),
                env: vec!["OPENAI_API_KEY".to_owned()],
                npm: Some("@ai-sdk/openai".to_owned()),
                models: Vec::new(),
            }],
            selected: 0,
            filter: String::new(),
            custom_id: String::new(),
            custom_base: String::new(),
            api_key: String::new(),
            format: blora_catalog::MessageFormat::Openai,
            preview_models: Vec::new(),
            resolved_base: String::new(),
            error: None,
            fullscreen: false,
            minimized: false,
        };
        let (lines, hits) = render_add_provider_with_hits(Rect::new(0, 0, 80, 24), &dialog, None);
        assert!(
            lines
                .iter()
                .any(|line| flatten(line).contains("+列表中没有我想要的供应商"))
        );
        assert!(hits.add_provider_rows.len() >= 2);
        assert_eq!(
            hits.hit(hits.add_provider_rows[0].0.x, hits.add_provider_rows[0].0.y),
            Some(Hit::AddProviderRow(0))
        );
    }

    #[test]
    fn add_provider_filter_narrows_catalog() {
        let mut dialog = AddProviderDialog {
            step: AddProviderStep::Catalog,
            catalog: vec![
                blora_catalog::CatalogEntry {
                    id: "openai".to_owned(),
                    name: "AcmeAI".to_owned(),
                    api: None,
                    env: Vec::new(),
                    npm: None,
                    models: vec![blora_catalog::CatalogModel {
                        id: "gpt-4o".to_owned(),
                        name: "GPT-4o".to_owned(),
                        context: None,
                    }],
                },
                blora_catalog::CatalogEntry {
                    id: "deepseek".to_owned(),
                    name: "DeepSeek".to_owned(),
                    api: None,
                    env: Vec::new(),
                    npm: None,
                    models: Vec::new(),
                },
            ],
            selected: 1,
            filter: "deep".to_owned(),
            custom_id: String::new(),
            custom_base: String::new(),
            api_key: String::new(),
            format: blora_catalog::MessageFormat::Openai,
            preview_models: Vec::new(),
            resolved_base: String::new(),
            error: None,
            fullscreen: false,
            minimized: false,
        };
        dialog.clamp_catalog_selected();
        let (lines, _) = render_add_provider_with_hits(Rect::new(0, 0, 80, 24), &dialog, None);
        let joined: String = lines.iter().map(|line| flatten(line)).collect();
        assert!(
            joined.contains("DeepSeek"),
            "filtered catalog keeps DeepSeek"
        );
        assert!(
            !joined.contains("AcmeAI"),
            "filtered catalog hides AcmeAI, got {joined}"
        );
        assert!(joined.contains("筛选"));
    }

    #[test]
    fn add_provider_review_lists_models_and_base() {
        let dialog = AddProviderDialog {
            step: AddProviderStep::Review,
            catalog: Vec::new(),
            selected: 0,
            filter: String::new(),
            custom_id: "crewrouter".to_owned(),
            custom_base: "https://router.bloret.net".to_owned(),
            api_key: "sk-test".to_owned(),
            format: blora_catalog::MessageFormat::Openai,
            preview_models: vec![
                blora_catalog::CatalogModel {
                    id: "fusion".to_owned(),
                    name: "fusion".to_owned(),
                    context: None,
                },
                blora_catalog::CatalogModel {
                    id: "crew-router".to_owned(),
                    name: "crew-router".to_owned(),
                    context: None,
                },
            ],
            resolved_base: "https://router.bloret.net/v1".to_owned(),
            error: None,
            fullscreen: false,
            minimized: false,
        };
        let (lines, _) = render_add_provider_with_hits(Rect::new(0, 0, 80, 24), &dialog, None);
        let joined: String = lines.iter().map(|line| flatten(line)).collect();
        assert!(joined.contains("https://router.bloret.net/v1"));
        assert!(joined.contains("fusion"));
        assert!(joined.contains("OpenAIChatCompletions"));
        assert!(joined.contains("enter保存"));
    }

    #[test]
    fn provider_dialog_retains_controls_on_compact_screens() {
        let dialog = sample_provider_dialog();
        let (lines, hits) = render_provider_with_hits(Rect::new(0, 0, 20, 4), &dialog, None);
        assert!(lines.iter().any(|line| !line.trim().is_empty()));
        assert!(hits.traffic_lights[0].is_some());
    }

    fn sample_theme_dialog() -> ThemeDialog {
        ThemeDialog {
            options: vec![
                ThemeOption {
                    id: "coral".to_owned(),
                    display: "珊瑚".to_owned(),
                    hint: "当前 · 深靛灰与柔和珊瑚红".to_owned(),
                    swatches: vec![],
                },
                ThemeOption {
                    id: "indigo".to_owned(),
                    display: "靛蓝".to_owned(),
                    hint: "冷灰基底与沉静蓝".to_owned(),
                    swatches: vec![],
                },
                ThemeOption {
                    id: "graphite".to_owned(),
                    display: "石墨".to_owned(),
                    hint: "冷灰界面与低饱和钢蓝".to_owned(),
                    swatches: vec![],
                },
                ThemeOption {
                    id: "mono".to_owned(),
                    display: "单色".to_owned(),
                    hint: "纯中性灰与近黑主色".to_owned(),
                    swatches: vec![],
                },
                ThemeOption {
                    id: "circuit".to_owned(),
                    display: "电路".to_owned(),
                    hint: "碳灰界面与克制青色".to_owned(),
                    swatches: vec![],
                },
                ThemeOption {
                    id: "dusk".to_owned(),
                    display: "暮色".to_owned(),
                    hint: "暮色灰紫".to_owned(),
                    swatches: vec![],
                },
            ],
            selected: 0,
            scheme: Scheme::Light,
            original: ThemePref {
                palette: Palette::Coral,
                scheme: Scheme::Light,
            },
            fullscreen: false,
            minimized: false,
        }
    }

    fn render_theme_with_hits(
        area: Rect,
        dialog: &ThemeDialog,
        pointer: Option<(u16, u16)>,
    ) -> (Vec<String>, HitMap) {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| {
                hits = render_theme_dialog(frame, area, dialog, pointer, &Theme::dark());
            })
            .unwrap();
        let mut lines = Vec::new();
        for row in 0..area.height {
            let mut line = String::new();
            for col in 0..area.width {
                let cell = terminal
                    .backend()
                    .buffer()
                    .cell((col, row))
                    .cloned()
                    .unwrap_or_default();
                line.push_str(cell.symbol());
            }
            lines.push(line);
        }
        (lines, hits)
    }

    #[test]
    fn theme_dialog_renders_rows_and_marks_selection() {
        let dialog = sample_theme_dialog();
        let area = Rect::new(0, 0, 80, 24);
        let (lines, hits) = render_theme_with_hits(area, &dialog, None);
        assert_eq!(hits.theme_rows.len(), 6);
        let find_row = |needle: &str| {
            lines
                .iter()
                .position(|line| flatten(line).contains(needle))
                .unwrap_or_else(|| panic!("{needle} rendered"))
        };
        let title_row = find_row("主题配色");
        let auto_tab = find_row("自动");
        let light_tab = find_row("浅色");
        let dark_tab = find_row("深色");
        let coral_row = find_row("珊瑚");
        let indigo_row = find_row("靛蓝");
        assert!(auto_tab > title_row);
        assert_eq!(auto_tab, light_tab);
        assert_eq!(light_tab, dark_tab);
        assert!(coral_row > auto_tab);
        assert!(indigo_row > coral_row);
        assert_eq!(hits.theme_tabs.len(), 3);
        assert_eq!(
            hits.hit(hits.theme_tabs[0].0.x, hits.theme_tabs[0].0.y),
            Some(Hit::ThemeTab(0))
        );
        assert_eq!(
            hits.hit(hits.theme_tabs[2].0.x, hits.theme_tabs[2].0.y),
            Some(Hit::ThemeTab(2))
        );
        assert!(
            lines[coral_row].contains('❯'),
            "selected coral row has ❯, got {}",
            lines[coral_row]
        );
        assert!(
            !lines[indigo_row].contains('❯'),
            "unselected indigo row has no ❯, got {}",
            lines[indigo_row]
        );
        for (rect, index) in &hits.theme_rows {
            assert_eq!(hits.hit(rect.x, rect.y), Some(Hit::ThemeRow(*index)));
        }
    }

    #[test]
    fn theme_dialog_retains_controls_on_compact_screens() {
        let dialog = sample_theme_dialog();
        let (lines, hits) = render_theme_with_hits(Rect::new(0, 0, 20, 4), &dialog, None);
        assert!(lines.iter().any(|line| !line.trim().is_empty()));
        assert!(hits.traffic_lights[0].is_some());
    }

    #[test]
    fn theme_dialog_traffic_lights_are_clickable() {
        let dialog = sample_theme_dialog();
        let area = Rect::new(0, 0, 80, 24);
        let (lines, hits) = render_theme_with_hits(area, &dialog, None);
        let cases: [(usize, Hit); 3] = [
            (0, Hit::TrafficClose),
            (1, Hit::TrafficMinimize),
            (2, Hit::TrafficOpenBrowser),
        ];
        for (index, expected) in cases {
            let rect = hits.traffic_lights[index].expect("hit area");
            assert_eq!(hits.hit(rect.x, rect.y), Some(expected));
        }
        let red = hits.traffic_lights[0].expect("red");
        let row_chars = |line: &str| -> Vec<char> { line.chars().collect() };
        assert_eq!(row_chars(&lines[red.y as usize])[red.x as usize], '●');
        let (lines, _) = render_theme_with_hits(area, &dialog, Some((red.x, red.y)));
        assert_eq!(
            row_chars(&lines[red.y as usize])[red.x as usize],
            '✕',
            "red shows ✕ on hover"
        );
    }

    #[test]
    fn theme_dialog_minimize_hides_rows_fullscreen_grows() {
        let mut dialog = sample_theme_dialog();
        let area = Rect::new(0, 0, 80, 24);
        dialog.minimized = true;
        let (lines, hits) = render_theme_with_hits(area, &dialog, None);
        assert!(hits.theme_rows.is_empty(), "minimized has no option rows");
        assert!(hits.traffic_lights[0].is_some());
        assert!(
            lines.iter().any(|line| flatten(line).contains("主题配色")),
            "title bar remains"
        );
        assert!(
            !lines.iter().any(|line| flatten(line).contains("靛蓝")),
            "option names are hidden while minimized"
        );

        dialog.minimized = false;
        dialog.fullscreen = true;
        let (_, hits) = render_theme_with_hits(area, &dialog, None);
        assert_eq!(hits.theme_rows.len(), 6);
        let first = hits.theme_rows[0].0;
        // Fullscreen inner content starts near the terminal edge, not mid-screen.
        assert!(first.x <= 4, "fullscreen hugs the left, x={}", first.x);
    }

    #[test]
    fn wraps_on_width() {
        let lines = wrap_text("abcdef", 3);
        assert_eq!(lines, vec!["abc", "def"]);
        let lines = wrap_text("a\n\nb", 8);
        assert_eq!(lines, vec!["a", "", "b"]);
    }

    #[test]
    fn formats_token_counts() {
        assert_eq!(fmt_tokens(12), "12");
        assert_eq!(fmt_tokens(1200), "1.2K");
        assert_eq!(fmt_tokens(12_000), "12K");
        assert_eq!(fmt_tokens(1_200_000), "1.2M");
    }

    #[test]
    fn header_items_fit_mobile_columns_and_visible_hit_regions() {
        for width in [0, 1, 12, 24, 36, 40, 60, 80, 120] {
            let area = Rect::new(2, 1, width, 1);
            let mut line = Line::default();
            for text in [
                "Code",
                "‹ 1/42 ›",
                "手机项目-CrewRo+19411–5171",
                "一个很长的会话标题",
            ] {
                if let Some(hit) = header_item(&mut line, area, " · ", text, Style::default(), true)
                {
                    assert!(hit.width > 0);
                    assert!(hit.x >= area.x);
                    assert!(hit.right() <= area.right());
                }
                assert!(line.width() <= usize::from(width));
            }
        }
        let area = Rect::new(0, 0, 10, 1);
        let mut line = Line::from("Code");
        assert!(header_item(&mut line, area, " · ", "‹ 1/42 ›", Style::default(), false).is_none());
        assert_eq!(line.width(), 4);
    }

    #[test]
    fn mobile_canvas_keeps_controls_visible_and_slash_hits_aligned() {
        let mut model = FrameModel {
            dialog_stack: &[],
            workspace: Path::new("/tmp"),
            sessions: &[],
            index: 0,
            sidebar: &SidebarState::default(),
            right_panel: None,
            projection: None,
            pending: &[],
            input: "手机终端输入一段很长的中文消息用于验证光标",
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
            mentions: None,
            search: None,
            hide_tools: false,
            message_roles: true,
            scroll: 0,
            permission_mode: PermissionMode::Ask,
            model: "a-very-long-model-name-that-exceeds-the-phone-width",
            provider: "provider",
            mode_menu: None,
            project_picker: None,
            folder_picker: None,
            question_dialog: None,
            session_picker: None,
            git_dialog: None,
            git_status: None,
            user_label: "you",
            running: false,
            tick: 0,
            pointer: None,
            text_selection: None,
        };
        let commands = slash::matches("/");
        for (width, height) in [(24, 10), (36, 12), (40, 20), (60, 16), (120, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut cache = TranscriptCache::default();
            for show_menu in [false, true] {
                model.input = if show_menu {
                    "/"
                } else {
                    "手机终端输入一段很长的中文消息用于验证光标"
                };
                model.slash_hits = if show_menu { &commands } else { &[] };
                model.slash_selected = commands.len().saturating_sub(1);
                let mut hits = HitMap::default();
                terminal
                    .draw(|frame| {
                        hits = draw(frame, &model, &mut cache);
                    })
                    .unwrap();
                assert!(hits.composer.height >= 2);
                assert!(hits.transcript.height >= 3);
                for rect in hits
                    .hints
                    .iter()
                    .map(|(rect, _)| *rect)
                    .chain(hits.provider_target)
                    .chain(hits.toggle_approve)
                {
                    assert!(rect.width > 0 && rect.height > 0);
                    assert!(rect.right() <= width && rect.bottom() <= height);
                }
                assert!(hits.toggle_approve.is_some());
                assert!(hits.provider_target.is_some());
                if show_menu {
                    let overlay = hits.overlay.unwrap();
                    assert!(
                        hits.slash_rows
                            .iter()
                            .any(|(_, index)| *index == model.slash_selected)
                    );
                    for (rect, index) in &hits.slash_rows {
                        assert!(rect.bottom() <= overlay.bottom());
                        let text: String = (rect.x..rect.right())
                            .map(|x| terminal.backend().buffer()[(x, rect.y)].symbol())
                            .collect();
                        assert!(text.contains(commands[*index].name), "{text}");
                    }
                }
            }
            model.input = "";
            model.slash_hits = &[];
            model.running = true;
            let mut hits = HitMap::default();
            terminal
                .draw(|frame| hits = draw(frame, &model, &mut cache))
                .unwrap();
            let (send, _) = hits
                .hints
                .iter()
                .find(|(_, action)| matches!(action, HintAction::Send))
                .unwrap();
            let label: String = (send.x..send.right())
                .map(|x| terminal.backend().buffer()[(x, send.y)].symbol())
                .collect();
            assert!(label.contains("排") || label.contains("queue"), "{label}");
            assert!(matches!(
                hits.hit(send.x, send.y),
                Some(Hit::Hint(HintAction::Send))
            ));
            model.running = false;
        }
    }

    #[test]
    fn mobile_provider_lists_stack_and_follow_selection() {
        let mut dialog = sample_provider_dialog();
        for index in 0..12 {
            let mut option = dialog.options[0].clone();
            option.display = format!("provider-{index}");
            dialog.options.push(option);
        }
        dialog.selected = dialog.options.len() - 1;
        dialog.model_selected = dialog.current_models().len().saturating_sub(1);
        for size in [(24, 10), (36, 12), (40, 24), (60, 16)] {
            let (_, hits) =
                render_provider_with_hits(Rect::new(0, 0, size.0, size.1), &dialog, None);
            let provider = hits
                .provider_rows
                .iter()
                .find(|(_, i)| *i == dialog.selected)
                .unwrap()
                .0;
            let model = hits
                .provider_model_rows
                .iter()
                .find(|(_, i)| *i == dialog.model_selected)
                .unwrap()
                .0;
            assert_eq!(provider.x, model.x);
            assert_eq!(provider.width, model.width);
            assert!(provider.bottom() < model.y);
            assert!(model.right() <= size.0 && model.bottom() <= size.1);
        }
    }

    #[test]
    fn chinese_input_tail_fits_without_splitting_wide_characters() {
        assert_eq!(visible_input("你好手机", 3), ("机".into(), 2));
        assert_eq!(visible_input("手机", 1), (String::new(), 0));
        assert_eq!(visible_input("手机a", 3), ("机a".into(), 3));
    }

    #[test]
    fn ellipsizes_long_text() {
        assert_eq!(ellipsize("hello", 10), "hello");
        assert!(ellipsize("hello world", 8).ends_with('…'));
        assert!(ellipsize("hello world", 8).width() <= 8);
    }

    #[test]
    fn visible_input_keeps_tail() {
        let (shown, col) = visible_input("abcdefghij", 4);
        assert_eq!(shown, "ghij");
        assert_eq!(col, 4);
        let (shown, col) = visible_input("ab", 8);
        assert_eq!(shown, "ab");
        assert_eq!(col, 2);
    }

    #[test]
    fn hitmap_prefers_slash_then_buttons() {
        let mut hits = HitMap {
            transcript: Rect::new(0, 0, 80, 20),
            composer: Rect::new(0, 21, 80, 2),
            slash_open: true,
            overlay: Some(Rect::new(2, 10, 40, 4)),
            ..HitMap::default()
        };
        hits.slash_rows.push((Rect::new(2, 11, 40, 1), 3));
        hits.allow = Some(Rect::new(40, 9, 7, 1));
        assert_eq!(hits.hit(5, 11), Some(Hit::Slash(3)));
        assert_eq!(hits.hit(42, 9), Some(Hit::Allow));
        assert_eq!(hits.hit(4, 4), Some(Hit::Transcript));
        assert_eq!(hits.hit(4, 21), Some(Hit::Composer));
        assert!(hits.over_slash(5, 11));
        assert!(!hits.over_slash(4, 4));
    }

    #[test]
    fn project_picker_rows_resolve_to_project_switch_targets() {
        let mut hits = HitMap::default();
        hits.project_picker_rows.push((Rect::new(4, 5, 20, 1), 0));
        assert_eq!(hits.hit(5, 5), Some(Hit::ProjectPickerRow(0)));
    }
}

#[cfg(test)]
mod dialog_layer_tests {
    use super::*;

    #[test]
    fn fractional_depth_and_window_visibility_are_continuous_and_clipped() {
        for dark in [true, false] {
            let theme = Theme::of(theme::Palette::Coral, dark);
            let rect = Rect::new(5, 4, 4, 2);
            let mut raw = ratatui::buffer::Buffer::empty(rect);
            raw[(5, 4)]
                .set_symbol("标题")
                .set_fg(theme.rose)
                .set_bg(theme.bg);
            let mut previous = None;
            for weight in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let mut layer = raw.clone();
                blend_region(&mut layer, rect, 0.35 + 0.65 * weight, &theme);
                if let Some(previous) = previous {
                    assert_ne!(layer[(5, 4)].fg, previous);
                }
                previous = Some(layer[(5, 4)].fg);
                assert_eq!(layer[(5, 4)].bg, theme.bg);
                assert_eq!(layer[(5, 4)].symbol(), "标题");
                let mut redraw = raw.clone();
                blend_region(&mut redraw, rect, 0.35 + 0.65 * weight, &theme);
                assert_eq!(layer, redraw);
                let mut clipped = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 6, 5));
                copy_region(&layer, &mut clipped);
                assert_eq!(clipped[(5, 4)], layer[(5, 4)]);
            }
            let mut half = raw.clone();
            let mut full = raw.clone();
            dim_region(&mut half, rect, 0.5, &theme);
            dim_region(&mut full, rect, 1.0, &theme);
            assert_ne!(half[(5, 4)].fg, full[(5, 4)].fg);
            assert_ne!(half[(5, 4)].fg, raw[(5, 4)].fg);
        }
    }

    #[test]
    fn cached_layers_retheme_without_mutating_original_colors() {
        let old = Theme::of(theme::Palette::Coral, true);
        let new = Theme::of(theme::Palette::Indigo, false);
        let rect = Rect::new(0, 0, 1, 1);
        let mut raw = ratatui::buffer::Buffer::empty(rect);
        raw[(0, 0)].set_fg(old.rose).set_bg(old.bg);
        let mut layer = raw.clone();
        retheme_layer(&mut layer, old, new);
        assert_eq!(layer[(0, 0)].fg, new.rose);
        assert_eq!(layer[(0, 0)].bg, new.bg);
        assert_eq!(raw[(0, 0)].fg, old.rose);
    }

    #[test]
    fn independent_onboarding_keeps_original_layer_after_business_drop() {
        use crate::dialogs::{DialogVisuals, TRANSITION_DURATION};
        use ratatui::{Terminal, backend::TestBackend};
        use std::time::{Duration, Instant};
        for (width, height) in [(100, 32), (24, 8), (3, 2)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut guide = crate::onboarding::Onboarding::new(false);
            let mut visuals = DialogVisuals::default();
            let mut cache = TranscriptCache::default();
            let start = Instant::now();
            visuals.sync(&[DialogId::Onboarding], start);
            let mut frames = Vec::new();
            for elapsed in [Duration::ZERO, TRANSITION_DURATION / 2, TRANSITION_DURATION] {
                terminal
                    .draw(|frame| {
                        draw_animated(
                            frame,
                            None,
                            &mut cache,
                            &mut visuals,
                            start + elapsed,
                            Some(&mut guide),
                            "测试账号",
                            false,
                        );
                    })
                    .unwrap();
                frames.push(terminal.backend().buffer().clone());
            }
            assert_ne!(frames[0], frames[1]);
            assert_ne!(frames[1], frames[2]);
            let raw = visuals.layers[0].layer.clone().unwrap();
            let close = start + TRANSITION_DURATION;
            visuals.sync(&[], close);
            drop(guide);
            for elapsed in [Duration::ZERO, TRANSITION_DURATION / 2] {
                terminal
                    .draw(|frame| {
                        let hits = draw_animated(
                            frame,
                            None,
                            &mut cache,
                            &mut visuals,
                            close + elapsed,
                            None,
                            "测试账号",
                            false,
                        );
                        assert!(hits.modal);
                        assert!(hits.modal_area.is_none());
                    })
                    .unwrap();
                assert_eq!(visuals.layers[0].layer.as_ref(), Some(&raw));
                if elapsed.is_zero() {
                    assert_eq!(terminal.backend().buffer(), &frames[2]);
                }
            }
            assert_eq!(terminal.backend().buffer(), &frames[1]);
            visuals.sync(&[], close + TRANSITION_DURATION);
            assert!(visuals.layers.is_empty());
        }
    }

    #[test]
    fn dimming_tracks_depth_and_preserves_background_for_both_schemes() {
        for dark in [true, false] {
            let theme = Theme::of(theme::Palette::Coral, dark);
            let area = Rect::new(0, 0, 2, 1);
            let mut original = ratatui::buffer::Buffer::empty(area);
            original[(0, 0)]
                .set_symbol("字")
                .set_fg(theme.rose)
                .set_bg(theme.bg)
                .set_style(Style::default().add_modifier(Modifier::BOLD));
            let mut one = original.clone();
            let mut two = original.clone();
            dim_region(&mut one, area, 1.0, &theme);
            dim_region(&mut two, area, 2.0, &theme);
            assert_ne!(one[(0, 0)].fg, original[(0, 0)].fg);
            assert_ne!(one[(0, 0)].fg, two[(0, 0)].fg);
            assert_eq!(two[(0, 0)].bg, theme.bg);
            assert_eq!(two[(0, 0)].symbol(), "字");
            assert!(!two[(0, 0)].modifier.contains(Modifier::BOLD));
            let mut next_frame = original.clone();
            dim_region(&mut next_frame, area, 2.0, &theme);
            assert_eq!(two, next_frame);
            let mut closed = original.clone();
            dim_region(&mut closed, area, 0.0, &theme);
            assert_eq!(closed, original);
            dim_region(&mut next_frame, area, 0.0, &theme);
            let mut deepest = original.clone();
            let mut deeper = original.clone();
            dim_region(&mut deepest, area, 16.0, &theme);
            dim_region(&mut deeper, area, 32.0, &theme);
            assert_eq!(deepest, deeper);
        }
    }

    #[test]
    fn dimming_only_changes_requested_layer_and_handles_terminal_colors() {
        let theme = Theme::of(theme::Palette::Coral, true);
        let area = Rect::new(0, 0, 4, 1);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        for (x, color) in [
            Color::Reset,
            Color::Indexed(200),
            Color::Indexed(240),
            Color::LightBlue,
        ]
        .into_iter()
        .enumerate()
        {
            buffer[(x as u16, 0)].set_fg(color);
        }
        let top = buffer[(3, 0)].clone();
        dim_region(&mut buffer, Rect::new(0, 0, 3, 1), 2.0, &theme);
        assert_eq!(buffer[(3, 0)], top);
        for x in 0..3 {
            assert!(matches!(buffer[(x, 0)].fg, Color::Rgb(..)));
        }
    }

    #[test]
    fn stacked_windows_draw_in_order_and_only_top_has_hits() {
        use ratatui::{Terminal, backend::TestBackend};
        let provider = ProviderDialog {
            options: vec![],
            selected: 0,
            pane: ProviderPane::Providers,
            model_selected: 0,
            fullscreen: false,
            minimized: false,
        };
        let settings = SettingsDialog {
            page: 0,
            selected: 0,
            fullscreen: false,
            minimized: false,
        };
        let mut model = FrameModel {
            dialog_stack: &[],
            workspace: Path::new("/tmp"),
            sessions: &[],
            index: 0,
            sidebar: &SidebarState::default(),
            right_panel: None,
            projection: None,
            pending: &[],
            input: "draft",
            status: "ready",
            notice: None,
            paste_preview: None,
            passport_dialog: None,
            provider_dialog: Some(&provider),
            add_provider_dialog: None,
            theme_dialog: None,
            context_dialog: None,
            settings_dialog: Some(&settings),
            harness_dialog: None,
            permission_dialog: None,
            tool_dialog: None,
            tool_detail_dialog: None,
            slash_hits: &[],
            slash_selected: 0,
            mentions: None,
            search: None,
            hide_tools: false,
            message_roles: true,
            scroll: 0,
            permission_mode: PermissionMode::Ask,
            model: "model",
            provider: "provider",
            mode_menu: None,
            project_picker: None,
            folder_picker: None,
            question_dialog: None,
            session_picker: None,
            git_dialog: None,
            git_status: None,
            user_label: "you",
            running: false,
            tick: 0,
            pointer: None,
            text_selection: None,
        };
        for (width, height) in [(120, 40), (40, 12)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut cache = TranscriptCache::default();
            let mut hits = HitMap::default();
            terminal
                .draw(|f| {
                    hits = draw(f, &model, &mut cache);
                })
                .unwrap();
            let base = terminal.backend().buffer().clone();
            model.dialog_stack = &[DialogId::Provider, DialogId::Settings];
            terminal
                .draw(|f| {
                    hits = draw(f, &model, &mut cache);
                })
                .unwrap();
            let stacked = terminal.backend().buffer().clone();
            assert!(hits.modal);
            assert!(!hits.settings_rows.is_empty());
            let close = hits.traffic_lights[0].expect("settings close control");
            assert_eq!(stacked[(close.x, close.y)].symbol(), "●");
            assert!(hits.settings_tabs.iter().all(|(rect, _)| rect.y > close.y));
            assert!(hits.provider_rows.is_empty());
            assert_eq!(hits.hit(0, 0), None);
            assert_ne!(base[(0, 0)].fg, stacked[(0, 0)].fg);
            terminal
                .draw(|f| {
                    hits = draw(f, &model, &mut cache);
                })
                .unwrap();
            assert_eq!(terminal.backend().buffer(), &stacked);
            let start = std::time::Instant::now();
            let mut visuals = crate::dialogs::DialogVisuals::default();
            visuals.sync(model.dialog_stack, start);
            let settled = start + crate::dialogs::TRANSITION_DURATION;
            terminal
                .draw(|frame| {
                    hits = draw_animated(
                        frame,
                        Some(&model),
                        &mut cache,
                        &mut visuals,
                        settled,
                        None,
                        "you",
                        false,
                    );
                })
                .unwrap();
            assert_eq!(terminal.backend().buffer(), &stacked);
            assert!(!hits.settings_rows.is_empty());
            let parent_raw = visuals.layers[0].layer.clone().unwrap();
            let child_raw = visuals.layers[1].layer.clone().unwrap();
            // Removing the business Option must not remove the closing visual layer.
            model.settings_dialog = None;
            visuals.sync(&[DialogId::Provider], settled);
            terminal
                .draw(|frame| {
                    hits = draw_animated(
                        frame,
                        Some(&model),
                        &mut cache,
                        &mut visuals,
                        settled,
                        None,
                        "you",
                        false,
                    );
                })
                .unwrap();
            assert_eq!(terminal.backend().buffer(), &stacked);
            assert!(hits.settings_rows.is_empty());
            assert!(hits.provider_rows.is_empty());
            assert!(hits.modal);
            let mid = settled + crate::dialogs::TRANSITION_DURATION / 2;
            terminal
                .draw(|frame| {
                    draw_animated(
                        frame,
                        Some(&model),
                        &mut cache,
                        &mut visuals,
                        mid,
                        None,
                        "you",
                        false,
                    );
                })
                .unwrap();
            assert_ne!(terminal.backend().buffer(), &stacked);
            assert_eq!(visuals.layers[0].layer.as_ref(), Some(&parent_raw));
            assert_eq!(visuals.layers[1].layer.as_ref(), Some(&child_raw));
            visuals.sync(
                &[DialogId::Provider],
                settled + crate::dialogs::TRANSITION_DURATION,
            );
            terminal
                .draw(|frame| {
                    hits = draw_animated(
                        frame,
                        Some(&model),
                        &mut cache,
                        &mut visuals,
                        settled + crate::dialogs::TRANSITION_DURATION,
                        None,
                        "you",
                        false,
                    );
                })
                .unwrap();
            assert_eq!(visuals.layers.len(), 1);
            assert_eq!(visuals.layers[0].layer.as_ref(), Some(&parent_raw));
            assert!(hits.settings_rows.is_empty());
            model.settings_dialog = Some(&settings);
            model.dialog_stack = &[DialogId::Settings, DialogId::Provider];
            terminal
                .draw(|f| {
                    hits = draw(f, &model, &mut cache);
                })
                .unwrap();
            assert!(hits.settings_rows.is_empty());
            assert!(hits.traffic_lights[0].is_some());
            model.dialog_stack = &[];
            terminal
                .draw(|f| {
                    hits = draw(f, &model, &mut cache);
                })
                .unwrap();
            let restored = terminal.backend().buffer();
            for y in base.area.y..base.area.bottom() {
                let mut x = base.area.x;
                while x < base.area.right() {
                    // Wide glyphs hide continuation cells in the terminal backend.
                    assert_eq!(restored[(x, y)], base[(x, y)]);
                    x += base[(x, y)].symbol().width().max(1) as u16;
                }
            }
        }
    }

    #[test]
    fn isolated_dialog_map_has_no_page_targets() {
        let map = HitMap {
            modal: true,
            ..HitMap::default()
        };
        assert_eq!(map.hit(20, 10), None);
        assert!(!map.over_slash(20, 10));
    }
}
