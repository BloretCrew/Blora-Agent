// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Frame layout: open canvas, focused composer, dim chrome.

use std::path::Path;

use blora_session::{SessionProjection, TranscriptItem};
use blora_storage::{ApprovalRecord, SessionSummary};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::slash::{self, SlashCommand};
use crate::theme::{Palette, Scheme, Theme, ThemePref};

const PAD: u16 = 2;
/// Work indicator: ping-pong through this star sequence.
const SPINNER: &[char] = &[
    '✦', '✧', '✩', '✪', '✫', '✬', '✭', '✮', '✯', '✰', '✴', '✵', '✶', '✷', '✸',
    '✹', '✺', '✻', '✼', '✽',
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

pub struct FrameModel<'a> {
    pub workspace: &'a Path,
    pub sessions: &'a [SessionSummary],
    pub index: usize,
    pub projection: Option<&'a SessionProjection>,
    pub pending: &'a [ApprovalRecord],
    pub input: &'a str,
    pub status: &'a str,
    pub notice: Option<&'a str>,
    /// Pending PassPort login; renders as a centered modal dialog.
    pub passport_dialog: Option<&'a PassportDialog>,
    /// Model-provider switch dialog; renders as a centered modal dialog.
    pub provider_dialog: Option<&'a ProviderDialog>,
    /// Add-provider wizard; takes render priority over the switch dialog.
    pub add_provider_dialog: Option<&'a AddProviderDialog>,
    /// Color-scheme picker; renders as a centered modal dialog.
    pub theme_dialog: Option<&'a ThemeDialog>,
    pub slash_hits: &'a [&'static SlashCommand],
    pub slash_selected: usize,
    pub search: Option<&'a str>,
    pub hide_tools: bool,
    pub scroll: usize,
    pub auto_approve: bool,
    pub model: &'a str,
    pub provider: &'a str,
    pub user_label: &'a str,
    pub running: bool,
    pub tick: u64,
    pub pointer: Option<(u16, u16)>,
}

/// A clickable region from the last painted frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Transcript,
    Composer,
    Slash(usize),
    Allow,
    Deny,
    PrevSession,
    NextSession,
    CancelRun,
    ToggleApprove,
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
    pub transcript: Rect,
    pub composer: Rect,
    pub overlay: Option<Rect>,
    pub notice: bool,
    pub slash_open: bool,
    pub slash_rows: Vec<(Rect, usize)>,
    pub allow: Option<Rect>,
    pub deny: Option<Rect>,
    pub prev_session: Option<Rect>,
    pub next_session: Option<Rect>,
    pub cancel_run: Option<Rect>,
    pub toggle_approve: Option<Rect>,
    pub hints: Vec<(Rect, HintAction)>,
    /// macOS-style traffic-light dots of the login dialog.
    pub traffic_lights: [Option<Rect>; 3],
    /// Provider rows of the provider-switch dialog.
    pub provider_rows: Vec<(Rect, usize)>,
    /// Model rows of the provider-switch dialog.
    pub provider_model_rows: Vec<(Rect, usize)>,
    /// Catalog rows of the add-provider wizard.
    pub add_provider_rows: Vec<(Rect, usize)>,
    /// Scheme rows of the theme picker.
    pub theme_rows: Vec<(Rect, usize)>,
    /// Auto / light / dark section tabs.
    pub theme_tabs: Vec<(Rect, usize)>,
}

impl HitMap {
    #[must_use]
    pub fn hit(&self, col: u16, row: u16) -> Option<Hit> {
        for (rect, idx) in &self.slash_rows {
            if contains(*rect, col, row) {
                return Some(Hit::Slash(*idx));
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
        for (rect, idx) in &self.provider_rows {
            if contains(*rect, col, row) {
                return Some(Hit::ProviderRow(*idx));
            }
        }
        for (rect, idx) in &self.provider_model_rows {
            if contains(*rect, col, row) {
                return Some(Hit::ProviderModelRow(*idx));
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
        for (rect, idx) in &self.theme_rows {
            if contains(*rect, col, row) {
                return Some(Hit::ThemeRow(*idx));
            }
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

pub fn draw(frame: &mut Frame<'_>, model: &FrameModel<'_>) -> HitMap {
    let theme = Theme::current();
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.base()), area);

    let show_slash = slash::is_open(model.input);
    let show_notice = !show_slash && model.notice.is_some();
    let extra = if show_slash {
        u16::try_from(model.slash_hits.len().clamp(1, slash::MAX_VISIBLE)).unwrap_or(1)
    } else if show_notice {
        u16::try_from(model.notice.map(line_count).unwrap_or(1).clamp(1, 12)).unwrap_or(1)
    } else {
        0
    };
    let show_approval = !model.pending.is_empty();

    let mut constraints = vec![
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
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
    render_header(frame, header, model, &theme, &mut hits);
    render_rule(frame, rule, &theme);
    render_transcript(frame, transcript, model, &theme);
    if let Some(rect) = approval {
        render_approval(frame, rect, model, &theme, &mut hits);
    }
    if let Some(rect) = overlay {
        if show_slash {
            render_slash(frame, rect, model, &theme, &mut hits);
        } else if let Some(body) = model.notice {
            render_notice(frame, rect, body, &theme);
        }
    }
    render_composer(frame, composer, model, &theme, &mut hits);
    render_status(frame, status, model, &theme);
    render_hints(frame, hints, model, &theme, &mut hits);
    if let Some(dialog) = model.passport_dialog {
        let dialog_hits = render_passport_dialog(frame, area, dialog, model.pointer, &theme);
        hits.traffic_lights = dialog_hits.traffic_lights;
    } else if let Some(dialog) = model.add_provider_dialog {
        let dialog_hits = render_add_provider_dialog(frame, area, dialog, model.pointer, &theme);
        hits.add_provider_rows = dialog_hits.add_provider_rows;
        hits.traffic_lights = dialog_hits.traffic_lights;
    } else if let Some(dialog) = model.theme_dialog {
        let dialog_hits = render_theme_dialog(frame, area, dialog, model.pointer, &theme);
        hits.theme_rows = dialog_hits.theme_rows;
        hits.theme_tabs = dialog_hits.theme_tabs;
        hits.traffic_lights = dialog_hits.traffic_lights;
    } else if let Some(dialog) = model.provider_dialog {
        let dialog_hits = render_provider_dialog(frame, area, dialog, model.pointer, &theme);
        hits.provider_rows = dialog_hits.provider_rows;
        hits.provider_model_rows = dialog_hits.provider_model_rows;
        hits.traffic_lights = dialog_hits.traffic_lights;
    }
    hits
}

fn inset(area: Rect) -> Rect {
    let pad = if area.width > PAD * 2 + 8 { PAD } else { 1 };
    Rect {
        x: area.x.saturating_add(pad),
        y: area.y,
        width: area.width.saturating_sub(pad.saturating_mul(2)),
        height: area.height,
    }
}

/// Centered dialog frame. Yellow minimizes to a title bar; green fills the
/// terminal minus a one-cell margin.
fn dialog_outer(
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
        hits.traffic_lights[index] = Some(Rect {
            x: title_row.x.saturating_add(1 + (index as u16) * 2),
            y: title_row.y,
            width: 1,
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
    let id = session
        .map(|item| short_id(item.id.as_str()))
        .unwrap_or_else(|| "—".to_owned());
    let title = session
        .and_then(|item| item.title.clone())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| id.clone());
    let counter = format!("{}/{}", model.index + 1, model.sessions.len().max(1));
    let left = Line::from(vec![
        Span::styled("blora", theme.rose_bold()),
        Span::styled("  ·  ", theme.mute()),
        Span::styled(mode, theme.fg(theme.sage)),
        Span::styled("  ·  ", theme.mute()),
        Span::styled("‹ ", theme.dim()),
        Span::styled(counter.clone(), theme.dim()),
        Span::styled(" ›", theme.dim()),
        Span::styled("  ", theme.mute()),
        Span::styled(ellipsize(&title, 28), theme.dim()),
    ]);
    let mut x = inner.x
        + u16::try_from("blora".width() + "  ·  ".width() + mode.width() + "  ·  ".width())
            .unwrap_or(0);
    hits.prev_session = Some(Rect {
        x,
        y: inner.y,
        width: 1,
        height: 1,
    });
    x = x.saturating_add(2);
    x = x.saturating_add(
        u16::try_from(counter.width())
            .unwrap_or(0)
            .saturating_add(1),
    );
    hits.next_session = Some(Rect {
        x,
        y: inner.y,
        width: 1,
        height: 1,
    });
    let tokens = model
        .projection
        .map(|projection| {
            format!(
                "{} → {}",
                fmt_tokens(projection.input_tokens),
                fmt_tokens(projection.output_tokens)
            )
        })
        .unwrap_or_default();
    let right = if model.running {
        format!("{}  running", spinner(model.tick))
    } else {
        tokens
    };
    frame.render_widget(Paragraph::new(left).style(theme.base()), inner);
    if !right.is_empty() && inner.width > 24 {
        let width = right.width().min(inner.width as usize) as u16;
        let rect = Rect {
            x: inner.x + inner.width.saturating_sub(width),
            y: inner.y,
            width,
            height: 1,
        };
        if model.running {
            hits.cancel_run = Some(rect);
        }
        let style = if model.running {
            theme.fg(theme.sage)
        } else {
            theme.mute()
        };
        frame.render_widget(Paragraph::new(right).style(style), rect);
    }
}

fn render_rule(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let inner = inset(area);
    let line = "─".repeat(inner.width as usize);
    frame.render_widget(Paragraph::new(line).style(theme.fg(theme.hairline)), inner);
}

fn render_transcript(frame: &mut Frame<'_>, area: Rect, model: &FrameModel<'_>, theme: &Theme) {
    let inner = inset(area);
    let width = inner.width.saturating_sub(2) as usize;
    let lines = model.projection.map_or_else(
        || welcome_lines(theme),
        |projection| {
            let rendered = transcript_lines(
                projection,
                model.search,
                model.hide_tools,
                width.max(8),
                model.running,
                model.tick,
                model.user_label,
                theme,
            );
            if rendered.is_empty() {
                welcome_lines(theme)
            } else {
                rendered
            }
        },
    );
    let visible = transcript_window(lines, inner.height as usize, model.scroll);
    frame.render_widget(Paragraph::new(visible).style(theme.base()), inner);
}

fn welcome_lines(theme: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::default(),
        Line::from(Span::styled("blora", theme.rose_bold())),
        Line::from(Span::styled(
            "local-first code, work, and agent",
            theme.mute(),
        )),
        Line::default(),
        Line::from(Span::styled(
            "type a message, or / for commands",
            theme.dim(),
        )),
    ]
}

fn transcript_lines(
    projection: &SessionProjection,
    search: Option<&str>,
    hide_tools: bool,
    width: usize,
    running: bool,
    tick: u64,
    user_label: &str,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let needle = search.map(str::to_ascii_lowercase);
    let mut out = Vec::new();
    let visible: Vec<&TranscriptItem> = projection
        .transcript
        .iter()
        .filter(|item| {
            !(hide_tools && matches!(item, TranscriptItem::Tool { .. }))
                && item_matches(item, needle.as_deref())
        })
        .collect();
    let mut index = 0usize;
    while index < visible.len() {
        if !out.is_empty() {
            out.push(Line::default());
        }
        if matches!(visible[index], TranscriptItem::Tool { .. }) {
            let start = index;
            index += 1;
            while index < visible.len() && matches!(visible[index], TranscriptItem::Tool { .. }) {
                index += 1;
            }
            let group = &visible[start..index];
            let summary = blora_session::summarize_tool_run(group, running);
            let failed = group.iter().any(|item| matches!(item, TranscriptItem::Tool { status, .. } if status == "failed" || status == "error"));
            let color = if failed {
                theme.rust
            } else if running
                && group.iter().any(|item| {
                    matches!(item, TranscriptItem::Tool { status, .. } if status == "running" || status == "requested")
                })
            {
                theme.amber
            } else {
                theme.text_dim
            };
            out.push(Line::from(vec![
                Span::styled("  ", theme.mute()),
                Span::styled(summary, theme.fg(color)),
            ]));
            continue;
        }
        let item = visible[index];
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
                    theme,
                );
            }
            TranscriptItem::Assistant { text, .. } => {
                push_block(
                    &mut out,
                    "blora",
                    theme.sage,
                    text,
                    width,
                    needle.as_deref(),
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
    if let Some(live) = projection.live_assistant() {
        let show = match needle.as_deref() {
            None => true,
            Some(needle) => live.to_ascii_lowercase().contains(needle),
        };
        if show {
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
                theme,
            );
            if running {
                out.push(Line::from(vec![
                    Span::styled("  ", theme.mute()),
                    Span::styled(spinner(tick).to_string(), theme.fg(theme.sage)),
                ]));
            }
        }
    }
    out
}

/// Keep the latest lines in view. `scroll` is how many lines above the bottom
/// to reveal; 0 always shows the newest message.
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

fn transcript_window(
    lines: Vec<Line<'static>>,
    height: usize,
    scroll: usize,
) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    let total = lines.len();
    if total <= height {
        return lines;
    }
    let max_scroll = total - height;
    let from_bottom = scroll.min(max_scroll);
    let start = total - height - from_bottom;
    lines.into_iter().skip(start).take(height).collect()
}

fn push_block(
    out: &mut Vec<Line<'static>>,
    role: &str,
    accent: ratatui::style::Color,
    text: &str,
    width: usize,
    needle: Option<&str>,
    theme: &Theme,
) {
    out.push(Line::from(vec![
        Span::styled("┃ ", theme.fg(accent)),
        Span::styled(
            role.to_owned(),
            theme.fg(accent).add_modifier(Modifier::BOLD),
        ),
    ]));
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
    let summary = ellipsize(&first.summary, inner.width.saturating_sub(22) as usize);
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
    let start = model
        .slash_selected
        .saturating_sub(slash::MAX_VISIBLE.saturating_sub(1));
    let end = model.slash_hits.len().min(start + slash::MAX_VISIBLE);
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
    let [_, frame_x, _] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(dialog_width),
        Constraint::Min(0),
    ])
    .flex(Flex::Center)
    .areas(area);
    let [_, frame_y, _] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(DIALOG_HEIGHT),
        Constraint::Min(0),
    ])
    .flex(Flex::Center)
    .areas(frame_x);

    let inner = paint_dialog_chrome(frame, frame_y, theme);

    let rows: [Rect; 5] = split_dialog_rows(inner, DIALOG_HEIGHT - 2);
    let wrap_width = inner.width.saturating_sub(2) as usize;

    paint_traffic_title(
        frame,
        rows[0],
        "Bloret PassPort 登录",
        pointer,
        theme,
        &mut hits,
    );

    // Row 1: instructions.
    let instructions = wrap_text(
        "在浏览器打开下面的链接并输入设备码，授权后这里会自动登录。",
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
        "已打开浏览器"
    } else {
        "等待授权"
    };
    let hint = Line::from(vec![
        Span::styled("esc", theme.mute()),
        Span::styled(" 隐藏对话框 · ", theme.mute()),
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
    if dialog.options.is_empty() {
        return hits;
    }
    let left_count = dialog.options.len().clamp(1, 12) as u16;
    let compact_height = left_count.saturating_add(6);
    if !dialog.fullscreen
        && !dialog.minimized
        && (area.width < 48 || area.height < compact_height.min(12))
    {
        return hits;
    }
    let compact_width = area.width.saturating_sub(4).clamp(56, 88);
    let Some(frame_y) = dialog_outer(
        area,
        compact_width,
        compact_height.max(12),
        dialog.fullscreen,
        dialog.minimized,
    ) else {
        return hits;
    };

    let inner = paint_dialog_chrome(frame, frame_y, theme);
    if inner.height == 0 {
        return hits;
    }

    let [title_row, body, hint_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(inner);
    paint_traffic_title(
        frame,
        title_row,
        "选择供应商与模型",
        pointer,
        theme,
        &mut hits,
    );
    if dialog.minimized {
        return hits;
    }

    let [left, right] =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(body);
    let left_rows = split_n_rows(left, left_count.max(1));
    for (index, option) in dialog.options.iter().enumerate() {
        let rect = left_rows
            .get(index)
            .copied()
            .unwrap_or(Rect::new(left.x, left.y, 0, 0));
        if rect.height == 0 {
            continue;
        }
        hits.provider_rows.push((rect, index));
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
        let marker = if (is_selected && dialog.pane == ProviderPane::Providers) || hovered {
            "❯"
        } else if is_selected {
            "•"
        } else {
            " "
        };
        let name_style = if is_selected {
            theme.fg(theme.text)
        } else {
            theme.fg(theme.text_dim)
        };
        let hint = if option.id == blora_catalog::ADD_PROVIDER_ID {
            option.hint.clone()
        } else if option.available {
            option.hint.clone()
        } else {
            format!("不可用 · {}", option.hint)
        };
        let line = Line::from(vec![
            Span::styled(format!(" {marker} "), theme.fg(theme.rose)),
            Span::styled(option.display.clone(), name_style),
            Span::styled("  ", theme.mute()),
            Span::styled(hint, theme.mute()),
        ]);
        frame.render_widget(Paragraph::new(line).style(theme.base()), rect);
    }

    let models = dialog.current_models();
    if dialog.is_add() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  回车打开添加供应商向导",
                theme.mute(),
            )))
            .style(theme.base()),
            right,
        );
    } else if models.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("  没有可列出的模型", theme.mute())))
                .style(theme.base()),
            right,
        );
    } else {
        let right_rows = split_n_rows(right, models.len().clamp(1, 16) as u16);
        for (index, option) in models.iter().enumerate() {
            let rect = right_rows
                .get(index)
                .copied()
                .unwrap_or(Rect::new(right.x, right.y, 0, 0));
            if rect.height == 0 {
                continue;
            }
            hits.provider_model_rows.push((rect, index));
            let hovered = pointer
                .map(|(col, row)| contains(rect, col, row))
                .unwrap_or(false);
            let is_selected = index == dialog.model_selected;
            if is_selected || hovered {
                frame.render_widget(
                    Block::default().style(Style::default().bg(theme.bg_select)),
                    rect,
                );
            }
            let marker = if (is_selected && dialog.pane == ProviderPane::Models) || hovered {
                "❯"
            } else {
                " "
            };
            let name_style = if is_selected {
                theme.fg(theme.text)
            } else {
                theme.fg(theme.text_dim)
            };
            let line = Line::from(vec![
                Span::styled(format!(" {marker} "), theme.fg(theme.rose)),
                Span::styled(option.display.clone(), name_style),
                Span::styled("  ", theme.mute()),
                Span::styled(option.hint.clone(), theme.mute()),
            ]);
            frame.render_widget(Paragraph::new(line).style(theme.base()), rect);
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " ←/→ 切换栏  ·  enter 确认  ·  esc 关闭",
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
    if !dialog.fullscreen && !dialog.minimized && (area.width < 40 || area.height < 10) {
        return hits;
    }
    let Some(frame_y) = dialog_outer(
        area,
        64,
        compact_height,
        dialog.fullscreen,
        dialog.minimized,
    ) else {
        return hits;
    };
    let inner = paint_dialog_chrome(frame, frame_y, theme);
    if inner.height == 0 {
        return hits;
    }
    let [title_row, body, hint_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(inner);
    let title = match dialog.step {
        AddProviderStep::Catalog => "添加供应商 · 选择来源",
        AddProviderStep::CustomId => "添加供应商 · 标识",
        AddProviderStep::CustomBase => "添加供应商 · 接口地址",
        AddProviderStep::ApiKey => "添加供应商 · API 密钥",
        AddProviderStep::Review => "添加供应商 · 确认",
    };
    paint_traffic_title(frame, title_row, title, pointer, theme, &mut hits);
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
                let rect = rows
                    .get(vis)
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
                let shown = dialog.preview_models.len().clamp(1, list_body.height.max(1) as usize);
                let rows = split_n_rows(list_body, shown as u16);
                for (index, model) in dialog.preview_models.iter().take(shown).enumerate() {
                    let rect = rows
                        .get(index)
                        .copied()
                        .unwrap_or(Rect::new(list_body.x, list_body.y, 0, 0));
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
            AddProviderStep::Catalog => " 输入筛选  ·  enter 下一步  ·  esc 关闭",
            AddProviderStep::Review => " enter 保存  ·  ←→ 切换格式  ·  esc 返回",
            _ => " enter 下一步  ·  esc 上一步",
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
        Paragraph::new(Line::from(Span::styled(format!(" {hint}"), theme.mute()))).style(theme.base()),
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
    if !dialog.fullscreen
        && !dialog.minimized
        && (area.width < 30 || area.height < compact_height + 2)
    {
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

    let inner = paint_dialog_chrome(frame, frame_y, theme);

    if inner.height == 0 {
        return hits;
    }
    let mut rows = split_n_rows(inner, row_count + 1);
    let title_row = rows.remove(0);
    paint_traffic_title(frame, title_row, "主题配色", pointer, theme, &mut hits);
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
        height: 1,
    };
    let info = Rect {
        x: inner.x,
        y: inner.y.saturating_add(1),
        width: inner.width,
        height: 1,
    };
    let available = prompt.width.saturating_sub(2) as usize;
    let (visible, cursor_col) = visible_input(model.input, available);
    let prompt_line = if model.input.is_empty() {
        Line::from(vec![
            Span::styled("❯ ", theme.rose_bold()),
            Span::styled("message or /command", theme.mute()),
        ])
    } else {
        Line::from(vec![
            Span::styled("❯ ", theme.rose_bold()),
            Span::styled(visible, theme.base()),
        ])
    };
    frame.render_widget(Paragraph::new(prompt_line).style(theme.base()), prompt);

    let model_name = if model.model.is_empty() {
        "mock"
    } else {
        model.model
    };
    let perm = if model.auto_approve { "yolo" } else { "ask" };
    let provider = if model.provider.is_empty() {
        "—"
    } else {
        model.provider
    };
    let perm_x = inner.x + u16::try_from(2 + model_name.width() + "  ·  ".width()).unwrap_or(8);
    hits.toggle_approve = Some(Rect {
        x: perm_x,
        y: info.y,
        width: u16::try_from(perm.width()).unwrap_or(4),
        height: 1,
    });
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ", theme.base()),
            Span::styled(model_name, theme.fg(theme.sage)),
            Span::styled("  ·  ", theme.mute()),
            Span::styled(perm, theme.dim()),
            Span::styled("  ·  ", theme.mute()),
            Span::styled(provider, theme.mute()),
        ]))
        .style(theme.base()),
        info,
    );

    let cursor_x = prompt.x.saturating_add(2).saturating_add(cursor_col);
    if cursor_x < prompt.x.saturating_add(prompt.width) {
        frame.set_cursor_position(Position::new(cursor_x, prompt.y));
    }
}

fn render_status(frame: &mut Frame<'_>, area: Rect, model: &FrameModel<'_>, theme: &Theme) {
    let inner = inset(area);
    let cwd = model
        .workspace
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| model.workspace.display().to_string());
    let left = if model.status.is_empty() {
        format!("~/{}", ellipsize(&cwd, 24))
    } else {
        ellipsize(model.status, inner.width.saturating_sub(4) as usize)
    };
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
    let mut spans = Vec::new();
    let mut x = inner.x;
    for (i, (key, label, action)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("   ", theme.mute()));
            x = x.saturating_add(3);
        }
        let text = format!("{key} {label}");
        let width = u16::try_from(text.width()).unwrap_or(1);
        hits.hints.push((
            Rect {
                x,
                y: inner.y,
                width,
                height: 1,
            },
            *action,
        ));
        spans.push(Span::styled((*key).to_owned(), theme.dim()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled((*label).to_owned(), theme.mute()));
        x = x.saturating_add(width);
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.base()), inner);
}

fn visible_input(input: &str, width: usize) -> (String, u16) {
    if width == 0 {
        return (String::new(), 0);
    }
    let full = input.width();
    if full <= width {
        return (input.to_owned(), u16::try_from(full).unwrap_or(0));
    }
    let mut acc = 0usize;
    let mut start = 0;
    for (idx, ch) in input.char_indices().rev() {
        acc += ch.width().unwrap_or(0);
        start = idx;
        if acc >= width {
            break;
        }
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

#[must_use]
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

fn short_id(id: &str) -> String {
    let rest = id.split_once('_').map_or(id, |(_, rest)| rest);
    rest.chars().take(8).collect()
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
    let from_user = projection.and_then(|projection| {
        projection.transcript.iter().rev().find_map(|item| match item {
            TranscriptItem::User { text, .. } => text
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| ellipsize(line, 32)),
            _ => None,
        })
    });
    if let Some(text) = from_user {
        return text;
    }
    session_title
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn transcript_window_pins_newest_lines() {
        let lines: Vec<Line> = (0..10).map(|i| Line::from(i.to_string())).collect();
        let visible = transcript_window(lines.clone(), 4, 0);
        assert_eq!(
            visible.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec!["6", "7", "8", "9"]
        );
        let older = transcript_window(lines, 4, 3);
        assert_eq!(
            older.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec!["3", "4", "5", "6"]
        );
    }

    #[test]
    fn window_title_cycles_star_while_running() {
        assert_eq!(window_title(true, 0, "code", "问候"), "✦ Blora Code · 问候");
        assert_eq!(window_title(true, 19, "code", "问候"), "✽ Blora Code · 问候");
        assert_eq!(window_title(true, 20, "code", "问候"), "✼ Blora Code · 问候");
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
        let unavailable_row = &lines[openai_row];
        assert!(
            flatten(unavailable_row).contains("不可用"),
            "unavailable provider is labeled, got {unavailable_row}"
        );
        for (rect, index) in &hits.provider_rows {
            assert_eq!(hits.hit(rect.x, rect.y), Some(Hit::ProviderRow(*index)));
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
        assert!(joined.contains("DeepSeek"), "filtered catalog keeps DeepSeek");
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
                },
                blora_catalog::CatalogModel {
                    id: "crew-router".to_owned(),
                    name: "crew-router".to_owned(),
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
    fn provider_dialog_skips_tiny_screens() {
        let dialog = sample_provider_dialog();
        let (lines, hits) = render_provider_with_hits(Rect::new(0, 0, 20, 4), &dialog, None);
        assert!(lines.iter().all(|line| line.trim().is_empty()));
        assert!(hits.provider_rows.is_empty());
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
    fn theme_dialog_skips_tiny_screens() {
        let dialog = sample_theme_dialog();
        let (lines, hits) = render_theme_with_hits(Rect::new(0, 0, 20, 4), &dialog, None);
        assert!(lines.iter().all(|line| line.trim().is_empty()));
        assert!(hits.theme_rows.is_empty());
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
}
