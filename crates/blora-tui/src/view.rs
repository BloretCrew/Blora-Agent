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
use crate::theme::Theme;

const PAD: u16 = 2;
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Bloret PassPort device-flow login dialog shown while a login is pending.
#[derive(Clone, Debug)]
pub struct PassportDialog {
    pub user_code: String,
    pub verification_uri: String,
    /// True once the browser hand-off has been attempted; switches the footer.
    pub opened_browser: bool,
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
    pub slash_hits: &'a [&'static SlashCommand],
    pub slash_selected: usize,
    pub search: Option<&'a str>,
    pub hide_tools: bool,
    pub scroll: usize,
    pub auto_approve: bool,
    pub model: &'a str,
    pub provider: &'a str,
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
                model.scroll,
                model.search,
                model.hide_tools,
                width.max(8),
                theme,
            );
            if rendered.is_empty() {
                welcome_lines(theme)
            } else {
                rendered
            }
        },
    );
    frame.render_widget(Paragraph::new(lines).style(theme.base()), inner);
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
    scroll: usize,
    search: Option<&str>,
    hide_tools: bool,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let needle = search.map(str::to_ascii_lowercase);
    let filtered: Vec<_> = projection
        .transcript
        .iter()
        .filter(|item| !(hide_tools && matches!(item, TranscriptItem::Tool { .. })))
        .filter(|item| match needle.as_deref() {
            None => true,
            Some(needle) => match item {
                TranscriptItem::User { text, .. } | TranscriptItem::Assistant { text, .. } => {
                    text.to_ascii_lowercase().contains(needle)
                }
                TranscriptItem::Tool { name, .. } => name.to_ascii_lowercase().contains(needle),
                TranscriptItem::System { summary, .. } => {
                    summary.to_ascii_lowercase().contains(needle)
                }
            },
        })
        .collect();
    let total = filtered.len();
    let skip = scroll.min(total.saturating_sub(1));
    let mut out = Vec::new();
    for item in filtered
        .into_iter()
        .rev()
        .skip(skip)
        .take(48)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if !out.is_empty() {
            out.push(Line::default());
        }
        match item {
            TranscriptItem::User { text, .. } => {
                push_block(
                    &mut out,
                    "you",
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
            TranscriptItem::Tool { name, status, .. } => {
                let color = if status == "failed" || status == "error" {
                    theme.rust
                } else if status == "running" {
                    theme.amber
                } else {
                    theme.text_mute
                };
                out.push(Line::from(vec![
                    Span::styled("  ·  ", theme.mute()),
                    Span::styled(name.clone(), theme.fg(color)),
                    Span::styled(format!("  {status}"), theme.mute()),
                ]));
            }
            TranscriptItem::System { summary, .. } => {
                out.push(Line::from(vec![
                    Span::styled("  ", theme.mute()),
                    Span::styled(summary.clone(), theme.mute().add_modifier(Modifier::ITALIC)),
                ]));
            }
        }
    }
    out
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
    for line in wrap_text(text, body_width) {
        let mut spans = vec![Span::styled("  ", theme.fg(theme.text))];
        spans.extend(highlight_spans(&line, needle, theme));
        out.push(Line::from(spans));
    }
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

    frame.render_widget(Clear, frame_y);
    let block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(theme.fg(theme.hairline));
    let inner = block.inner(frame_y);
    frame.render_widget(block, frame_y);

    let rows: [Rect; 5] = split_dialog_rows(inner, DIALOG_HEIGHT - 2);
    let wrap_width = inner.width.saturating_sub(2) as usize;

    // Row 0: mac-style traffic lights (hovering shows their symbols), a
    // leading space for breathing room, then the dim title. Body text below
    // stays dim so the dots carry the visual weight.
    let title = "Bloret PassPort 登录";
    let dot_glyphs: [(&str, &str, Color); 3] = [
        ("●", "✕", theme.rust),
        ("●", "−", theme.amber),
        ("●", "+", theme.sage),
    ];
    // Click targets are deterministic: leading space, then each dot one blank
    // column apart. Compute them first so hover detection can use them.
    for (index, _) in dot_glyphs.iter().enumerate() {
        hits.traffic_lights[index] = Some(Rect {
            x: rows[0].x.saturating_add(1 + (index as u16) * 2),
            y: rows[0].y,
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
        title_line.push(Span::styled(glyph.to_owned(), theme.fg(*color)));
    }
    title_line.push(Span::styled("  ".to_owned(), theme.mute()));
    title_line.push(Span::styled(title.to_owned(), theme.fg(theme.text_dim)));
    let title_span_width = 1 + " ● ● ●".width() + "  ".width() + title.width();
    if title_span_width >= usize::from(rows[0].width) {
        // Too narrow for dots + title: keep the dots, drop the title text.
        title_line.truncate(6);
    }
    frame.render_widget(
        Paragraph::new(Line::from(title_line)).style(theme.base()),
        rows[0],
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

fn short_id(id: &str) -> String {
    let rest = id.split_once('_').map_or(id, |(_, rest)| rest);
    rest.chars().take(8).collect()
}

fn spinner(tick: u64) -> char {
    SPINNER[(tick as usize) % SPINNER.len()]
}

fn line_count(text: &str) -> usize {
    text.lines().count().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
