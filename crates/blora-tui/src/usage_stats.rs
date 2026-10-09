// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Aggregates recorded token usage for the context dialog's overview and model pages.

use std::collections::{BTreeMap, BTreeSet};

use blora_storage::{SessionSpan, UsageActivity};
use chrono::{Datelike, Duration, NaiveDate};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::i18n;
use crate::theme::Theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelUsage {
    pub name: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub daily: BTreeMap<NaiveDate, u64>,
}

impl ModelUsage {
    fn total(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

#[derive(Clone, Debug)]
pub struct UsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub sessions: usize,
    pub active_days: usize,
    pub span_days: usize,
    pub longest_session: Duration,
    pub longest_streak: usize,
    pub current_streak: usize,
    pub most_active: Option<(NaiveDate, u64)>,
    pub daily: BTreeMap<NaiveDate, u64>,
    pub models: Vec<ModelUsage>,
    pub range: usize,
    pub range_start: Option<NaiveDate>,
    pub today: NaiveDate,
}

impl UsageSummary {
    fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

pub struct RenderedStats {
    pub lines: Vec<Line<'static>>,
    pub range_line: Option<usize>,
    pub range_spans: [(usize, usize); 3],
}

pub fn page_label(page: usize) -> String {
    match page {
        0 => i18n::tr("context.page.current", "当前"),
        1 => i18n::tr("context.page.overview", "总览"),
        _ => i18n::tr("context.page.models", "模型"),
    }
}

pub fn hint(page: usize) -> String {
    if page == 0 {
        i18n::tr("context.hint.current", "←/→ 翻页 · ↑/↓ 滚动 · Esc 关闭")
    } else {
        i18n::tr(
            "context.hint.stats",
            "←/→ 翻页 · r 切换时间 · ↑/↓ 滚动 · Esc 关闭",
        )
    }
}

pub fn summarize(activity: &UsageActivity, range: usize, today: NaiveDate) -> UsageSummary {
    let range = range % 3;
    let range_start = match range {
        1 => today.checked_sub_signed(Duration::days(6)),
        2 => today.checked_sub_signed(Duration::days(29)),
        _ => None,
    };
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cached_tokens = 0u64;
    let mut daily: BTreeMap<NaiveDate, u64> = BTreeMap::new();
    let mut grouped: BTreeMap<String, ModelUsage> = BTreeMap::new();
    let mut session_ids = BTreeSet::new();
    for sample in &activity.samples {
        if !in_range(sample.day, range_start, today) {
            continue;
        }
        session_ids.insert(sample.session_id.clone());
        input_tokens = input_tokens.saturating_add(sample.input_tokens);
        output_tokens = output_tokens.saturating_add(sample.output_tokens);
        cached_tokens = cached_tokens.saturating_add(sample.cached_tokens);
        let tokens = sample.input_tokens.saturating_add(sample.output_tokens);
        daily
            .entry(sample.day)
            .and_modify(|total| *total = total.saturating_add(tokens))
            .or_insert(tokens);
        let model = grouped
            .entry(sample.model.clone())
            .or_insert_with(|| ModelUsage {
                name: sample.model.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cached_tokens: 0,
                daily: BTreeMap::new(),
            });
        model.input_tokens = model.input_tokens.saturating_add(sample.input_tokens);
        model.output_tokens = model.output_tokens.saturating_add(sample.output_tokens);
        model.cached_tokens = model.cached_tokens.saturating_add(sample.cached_tokens);
        model
            .daily
            .entry(sample.day)
            .and_modify(|total| *total = total.saturating_add(tokens))
            .or_insert(tokens);
    }
    for span in &activity.sessions {
        if in_range(span.updated_at.date_naive(), range_start, today) {
            session_ids.insert(span.id.clone());
        }
    }
    let mut models: Vec<ModelUsage> = grouped.into_values().collect();
    models.sort_by(|left, right| {
        right
            .total()
            .cmp(&left.total())
            .then_with(|| left.name.cmp(&right.name))
    });
    let active: Vec<NaiveDate> = daily
        .iter()
        .filter(|(_, tokens)| **tokens > 0)
        .map(|(day, _)| *day)
        .collect();
    let (longest_streak, current_streak) = streaks(&active, today);
    let most_active = daily
        .iter()
        .filter(|(_, tokens)| **tokens > 0)
        .max_by_key(|(_, tokens)| *tokens)
        .map(|(day, tokens)| (*day, *tokens));
    let span_days = match range_start {
        Some(start) => day_span(start, today),
        None => day_span(active.first().copied().unwrap_or(today), today),
    };
    UsageSummary {
        input_tokens,
        output_tokens,
        cached_tokens,
        sessions: session_ids.len(),
        active_days: active.len(),
        span_days,
        longest_session: longest_session(&activity.sessions, &session_ids),
        longest_streak,
        current_streak,
        most_active,
        daily,
        models,
        range,
        range_start,
        today,
    }
}

pub fn overview(summary: &UsageSummary, width: usize, theme: &Theme) -> RenderedStats {
    let width = width.max(1);
    let mut lines = heatmap(summary, width, theme)
        .into_iter()
        .map(|line| within(line, width))
        .collect::<Vec<_>>();
    lines.push(Line::default());
    let (range, range_spans) = range_line(summary.range, theme);
    let range_at = lines.len();
    lines.push(within(range, width));
    lines.push(Line::default());
    let favorite = summary
        .models
        .first()
        .map(|model| display_model(&model.name))
        .unwrap_or_else(|| "—".to_owned());
    let most_active = summary
        .most_active
        .map(|(day, _)| format_day(day))
        .unwrap_or_else(|| "—".to_owned());
    let rows = [
        (
            i18n::tr("context.favorite", "常用模型"),
            favorite,
            i18n::tr("context.total_tokens", "总 token"),
            format_tokens(summary.total_tokens()),
        ),
        (
            i18n::tr("context.sessions", "会话"),
            summary.sessions.to_string(),
            i18n::tr("context.longest_session", "最长会话"),
            format_duration(summary.longest_session),
        ),
        (
            i18n::tr("context.active_days", "活跃天数"),
            format!("{}/{}", summary.active_days, summary.span_days.max(1)),
            i18n::tr("context.longest_streak", "最长连续"),
            days_label(summary.longest_streak),
        ),
        (
            i18n::tr("context.most_active", "最活跃"),
            most_active,
            i18n::tr("context.current_streak", "当前连续"),
            days_label(summary.current_streak),
        ),
    ];
    if width < 52 {
        for (left_label, left_value, right_label, right_value) in &rows {
            lines.push(within(
                styled_stat(left_label, left_value, width, theme),
                width,
            ));
            lines.push(within(
                styled_stat(right_label, right_value, width, theme),
                width,
            ));
        }
    } else {
        for (left_label, left_value, right_label, right_value) in &rows {
            lines.push(stat_pair(
                left_label,
                left_value,
                right_label,
                right_value,
                width,
                theme,
            ));
        }
    }
    lines.push(within(
        Line::from(Span::styled(token_breakdown(summary), theme.dim())),
        width,
    ));
    if let Some(fun) = fun_line(summary.longest_session) {
        lines.push(Line::default());
        lines.push(within(
            Line::from(Span::styled(fun, theme.fg(theme.text_dim))),
            width,
        ));
    }
    if summary.total_tokens() == 0 && summary.sessions == 0 {
        lines.push(within(
            Line::from(Span::styled(
                i18n::tr(
                    "context.empty",
                    "还没有用量记录。完成一次对话后，这里会显示活跃情况。",
                ),
                theme.mute(),
            )),
            width,
        ));
    }
    RenderedStats {
        lines,
        range_line: Some(range_at),
        range_spans,
    }
}

pub fn models(summary: &UsageSummary, width: usize, theme: &Theme) -> RenderedStats {
    let width = width.max(1);
    let mut lines = vec![within(
        Line::from(Span::styled(
            i18n::tr("context.chart_title", "每日 Token"),
            theme.fg(theme.text).add_modifier(Modifier::BOLD),
        )),
        width,
    )];
    lines.extend(
        chart(summary, width, theme)
            .into_iter()
            .map(|line| within(line, width)),
    );
    lines.push(within(model_legend(summary, width, theme), width));
    lines.push(Line::default());
    let (range, range_spans) = range_line(summary.range, theme);
    let range_at = lines.len();
    lines.push(within(range, width));
    lines.push(Line::default());
    if summary.models.is_empty() {
        lines.push(within(
            Line::from(Span::styled(
                i18n::tr("context.no_models", "暂无模型用量"),
                theme.mute(),
            )),
            width,
        ));
    } else {
        lines.extend(model_details(summary, width, theme));
    }
    RenderedStats {
        lines,
        range_line: Some(range_at),
        range_spans,
    }
}

fn in_range(day: NaiveDate, start: Option<NaiveDate>, today: NaiveDate) -> bool {
    day <= today && start.is_none_or(|start| day >= start)
}

fn day_span(start: NaiveDate, end: NaiveDate) -> usize {
    let days = end
        .signed_duration_since(start)
        .num_days()
        .saturating_add(1);
    usize::try_from(days.max(1)).unwrap_or(1)
}

fn longest_session(spans: &[SessionSpan], ids: &BTreeSet<String>) -> Duration {
    spans
        .iter()
        .filter(|span| ids.contains(&span.id))
        .map(|span| span.updated_at.signed_duration_since(span.created_at))
        .filter(|duration| !duration.num_seconds().is_negative())
        .max()
        .unwrap_or(Duration::zero())
}

fn streaks(days: &[NaiveDate], today: NaiveDate) -> (usize, usize) {
    if days.is_empty() {
        return (0, 0);
    }
    let mut longest = 1usize;
    let mut run = 1usize;
    for pair in days.windows(2) {
        if pair[0].checked_add_signed(Duration::days(1)) == Some(pair[1]) {
            run += 1;
            longest = longest.max(run);
        } else if pair[1] != pair[0] {
            run = 1;
        }
    }
    let last = days[days.len() - 1];
    let yesterday = today.checked_sub_signed(Duration::days(1));
    if last != today && Some(last) != yesterday {
        return (longest, 0);
    }
    let present: BTreeSet<NaiveDate> = days.iter().copied().collect();
    let mut cursor = last;
    let mut current = 0usize;
    loop {
        if !present.contains(&cursor) {
            break;
        }
        current += 1;
        match cursor.checked_sub_signed(Duration::days(1)) {
            Some(previous) => cursor = previous,
            None => break,
        }
    }
    (longest, current)
}

fn heatmap(summary: &UsageSummary, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let gutter_width = 4;
    let (start, weeks, cell_width) = heatmap_layout(summary, width.saturating_sub(gutter_width));
    let mut peak = 1u64;
    for week in 0..weeks {
        for row in 0..7 {
            let Some(date) = cell_date(start, week, row) else {
                continue;
            };
            if date <= summary.today {
                peak = peak.max(summary.daily.get(&date).copied().unwrap_or(0));
            }
        }
    }
    let mut header = blank_cells(weeks * cell_width, theme.text_mute);
    let mut next_free = 0usize;
    for week in 0..weeks {
        let Some(date) = cell_date(start, week, 0) else {
            continue;
        };
        if week != 0 && date.day() > 7 {
            continue;
        }
        let label = month_label(date.month());
        let at = week * cell_width;
        if at >= next_free && write_text(&mut header, at, &label, theme.text_mute) {
            next_free = at + label.width() + 1;
        }
    }
    let mut lines = vec![cells_line(&gutter(""), theme.mute(), &header)];
    for row in 0..7 {
        let mut cells = blank_cells(weeks * cell_width, theme.text_mute);
        for week in 0..weeks {
            let Some(date) = cell_date(start, week, row) else {
                continue;
            };
            if date > summary.today {
                continue;
            }
            let value = summary.daily.get(&date).copied().unwrap_or(0);
            let level = heat_level(value, peak);
            let (ch, color) = if level == 0 {
                ('░', theme.text_mute)
            } else {
                ('█', heat_color(theme, level))
            };
            let origin = week * cell_width;
            for offset in 0..cell_width {
                cells[origin + offset] = Cell {
                    ch,
                    color,
                    skip: false,
                };
            }
        }
        lines.push(cells_line(&weekday_gutter(row), theme.mute(), &cells));
    }
    lines.push(legend_line(theme));
    lines
}

fn heatmap_layout(summary: &UsageSummary, inner: usize) -> (NaiveDate, usize, usize) {
    let inner = inner.max(1);
    let today_monday = monday_on_or_before(summary.today);
    let window_start = if summary.range == 0 {
        today_monday
            .checked_sub_signed(Duration::days(52 * 7))
            .unwrap_or(today_monday)
    } else {
        monday_on_or_before(summary.range_start.unwrap_or(summary.today))
    };
    let window_start = if window_start > today_monday {
        today_monday
    } else {
        window_start
    };
    let natural =
        usize::try_from(summary.today.signed_duration_since(window_start).num_days() / 7 + 1)
            .unwrap_or(1)
            .max(1);
    let cell_width = if natural <= 8 && inner / 2 >= natural {
        2
    } else {
        1
    };
    let weeks = natural.min((inner / cell_width).max(1));
    let start = if weeks < natural {
        today_monday
            .checked_sub_signed(Duration::days((weeks as i64 - 1) * 7))
            .unwrap_or(today_monday)
    } else {
        window_start
    };
    (start, weeks, cell_width)
}

fn cell_date(start: NaiveDate, week: usize, row: usize) -> Option<NaiveDate> {
    start.checked_add_signed(Duration::days(week as i64 * 7 + row as i64))
}

fn monday_on_or_before(date: NaiveDate) -> NaiveDate {
    let back = i64::from(date.weekday().num_days_from_monday());
    date.checked_sub_signed(Duration::days(back))
        .unwrap_or(date)
}

fn heat_level(value: u64, peak: u64) -> u8 {
    if value == 0 || peak == 0 {
        return 0;
    }
    let pct = value.saturating_mul(100) / peak.max(1);
    if pct > 75 {
        4
    } else if pct > 50 {
        3
    } else if pct > 25 {
        2
    } else {
        1
    }
}

fn heat_color(theme: &Theme, level: u8) -> Color {
    if !matches!(theme.rose, Color::Rgb(_, _, _)) {
        return match level {
            1 => theme.text_dim,
            2 => theme.amber,
            _ => theme.rose,
        };
    }
    match level {
        1 => mix(theme.bg, theme.amber, 0.7),
        2 => theme.amber,
        3 => mix(theme.amber, theme.rose, 0.45),
        _ => theme.rose,
    }
}

fn mix(from: Color, to: Color, toward: f32) -> Color {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (from, to) else {
        return to;
    };
    let blend = |start: u8, end: u8| -> u8 {
        let mixed = f32::from(start) + (f32::from(end) - f32::from(start)) * toward;
        mixed.round().clamp(0.0, 255.0) as u8
    };
    Color::Rgb(blend(ar, br), blend(ag, bg), blend(ab, bb))
}

fn legend_line(theme: &Theme) -> Line<'static> {
    let mut spans = vec![
        Span::styled(gutter(""), theme.mute()),
        Span::styled(i18n::tr("context.less", "少"), theme.mute()),
        Span::raw(" "),
    ];
    for level in 1..=4 {
        spans.push(Span::styled(
            "█".to_owned(),
            theme.fg(heat_color(theme, level)),
        ));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(i18n::tr("context.more", "多"), theme.mute()));
    Line::from(spans)
}

fn chart(summary: &UsageSummary, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let axis = 7usize;
    let plot_width = width.saturating_sub(axis).clamp(1, 64);
    let height = 7usize;
    let start = summary.range_start.unwrap_or_else(|| {
        summary
            .daily
            .keys()
            .next()
            .copied()
            .unwrap_or(summary.today)
    });
    let end = if summary.today < start {
        start
    } else {
        summary.today
    };
    let series: Vec<&ModelUsage> = summary.models.iter().take(4).collect();
    let mut slots: Vec<Vec<u64>> = series
        .iter()
        .map(|model| series_slots(&model.daily, start, end, plot_width))
        .collect();
    if slots.is_empty() {
        slots.push(vec![0; plot_width]);
    }
    let max = slots
        .iter()
        .flat_map(|row| row.iter().copied())
        .max()
        .unwrap_or(0)
        .max(1);
    let mut grid = vec![vec![Cell::blank(theme.text_mute); plot_width]; height];
    for (index, values) in slots.iter().enumerate() {
        let color = if series.is_empty() {
            theme.text_mute
        } else {
            series_color(theme, index)
        };
        paint_step(&mut grid, values, max, color);
    }
    let mut lines = Vec::with_capacity(height + 1);
    for row in 0..height {
        let label = if row == 0 {
            format_tokens(max)
        } else if row == height / 2 {
            format_tokens(max / 2)
        } else if row + 1 == height {
            "0".to_owned()
        } else {
            String::new()
        };
        let prefix = format!("{}│", axis_label(&label));
        lines.push(cells_line(&prefix, theme.mute(), &grid[row]));
    }
    lines.push(cells_line(
        &" ".repeat(axis),
        theme.mute(),
        &axis_dates(start, end, plot_width, theme),
    ));
    lines
}

fn series_slots(
    daily: &BTreeMap<NaiveDate, u64>,
    start: NaiveDate,
    end: NaiveDate,
    slots: usize,
) -> Vec<u64> {
    let mut values = vec![0u64; slots.max(1)];
    for (day, value) in daily {
        if *day < start || *day > end {
            continue;
        }
        let index = x_of(*day, start, end, values.len());
        values[index] = values[index].saturating_add(*value);
    }
    values
}

fn x_of(day: NaiveDate, start: NaiveDate, end: NaiveDate, slots: usize) -> usize {
    if slots <= 1 {
        return 0;
    }
    let span = end.signed_duration_since(start).num_days().max(1);
    let delta = day.signed_duration_since(start).num_days().clamp(0, span);
    let last = slots as i64 - 1;
    usize::try_from(delta * last / span)
        .unwrap_or(0)
        .min(slots - 1)
}

fn paint_step(grid: &mut [Vec<Cell>], values: &[u64], max: u64, color: Color) {
    let height = grid.len();
    let width = grid.first().map(Vec::len).unwrap_or(0);
    if height == 0 || width == 0 {
        return;
    }
    let mut previous: Option<(usize, usize)> = None;
    for (x, value) in values.iter().enumerate().take(width) {
        let row = row_from_top(*value, max, height);
        if let Some((from_x, from_row)) = previous {
            for column in from_x..=x {
                put(grid, from_row, column, '─', color);
            }
            let (low, high) = if from_row < row {
                (from_row, row)
            } else {
                (row, from_row)
            };
            for line in low..=high {
                put(grid, line, x, '│', color);
            }
        }
        put(grid, row, x, '●', color);
        previous = Some((x, row));
    }
}

fn row_from_top(value: u64, max: u64, height: usize) -> usize {
    if height <= 1 || max == 0 {
        return 0;
    }
    let span = height - 1;
    let scaled = value.saturating_mul(span as u64) / max;
    let from_bottom = usize::try_from(scaled).unwrap_or(0).min(span);
    span - from_bottom
}

fn put(grid: &mut [Vec<Cell>], row: usize, column: usize, ch: char, color: Color) {
    if let Some(line) = grid.get_mut(row)
        && let Some(cell) = line.get_mut(column)
    {
        *cell = Cell {
            ch,
            color,
            skip: false,
        };
    }
}

fn axis_label(text: &str) -> String {
    let fitted = fit(text, 6);
    let pad = 6usize.saturating_sub(fitted.width());
    format!("{}{fitted}", " ".repeat(pad))
}

fn axis_dates(start: NaiveDate, end: NaiveDate, slots: usize, theme: &Theme) -> Vec<Cell> {
    let mut cells = blank_cells(slots, theme.text_mute);
    let indexes = if slots < 12 {
        vec![0]
    } else {
        vec![0, slots / 2, slots.saturating_sub(1)]
    };
    let mut next_free = 0usize;
    for index in indexes {
        let date = slot_date(index, start, end, slots);
        let label = format!("{}/{}", date.month(), date.day());
        if index >= next_free && write_text(&mut cells, index, &label, theme.text_mute) {
            next_free = index + label.width() + 1;
        }
    }
    cells
}

fn slot_date(index: usize, start: NaiveDate, end: NaiveDate, slots: usize) -> NaiveDate {
    if slots <= 1 {
        return start;
    }
    let span = end.signed_duration_since(start).num_days().max(0);
    let delta = span * index as i64 / (slots as i64 - 1);
    start
        .checked_add_signed(Duration::days(delta))
        .unwrap_or(end)
}

fn model_legend(summary: &UsageSummary, width: usize, theme: &Theme) -> Line<'static> {
    if summary.models.is_empty() {
        return Line::from(Span::styled(
            i18n::tr("context.no_models", "暂无模型用量"),
            theme.mute(),
        ));
    }
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (index, model) in summary.models.iter().take(4).enumerate() {
        let name = fit(&display_model(&model.name), 18);
        let piece = format!("● {name}");
        let extra = if spans.is_empty() { 0 } else { 2 };
        if used + extra + piece.width() > width && !spans.is_empty() {
            break;
        }
        if extra > 0 {
            spans.push(Span::raw("  "));
            used += 2;
        }
        used += piece.width();
        spans.push(Span::styled(
            piece,
            theme
                .fg(series_color(theme, index))
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}

fn model_details(summary: &UsageSummary, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let total = summary.total_tokens();
    let shown = summary.models.iter().take(8).collect::<Vec<_>>();
    let mut lines = Vec::new();
    if width >= 64 {
        for (row_index, pair) in shown.chunks(2).enumerate() {
            let left_index = row_index * 2;
            let left = model_block(pair[0], total, left_index, theme);
            let right = pair
                .get(1)
                .map(|model| model_block(model, total, left_index + 1, theme));
            for row in 0..3 {
                let mut spans = left[row].spans.clone();
                if let Some(right) = &right {
                    let used = line_width(&left[row]);
                    let gap = (width / 2).saturating_sub(used).max(2);
                    spans.push(Span::raw(" ".repeat(gap)));
                    spans.extend(right[row].spans.clone());
                }
                lines.push(within(Line::from(spans), width));
            }
        }
    } else {
        for (index, model) in shown.iter().enumerate() {
            for line in model_block(model, total, index, theme) {
                lines.push(within(line, width));
            }
        }
    }
    if summary.models.len() > shown.len() {
        let extra = summary.models.len() - shown.len();
        lines.push(within(
            Line::from(Span::styled(
                i18n::tr("context.other_models", "另有 {n} 个模型")
                    .replace("{n}", &extra.to_string()),
                theme.mute(),
            )),
            width,
        ));
    }
    lines
}

fn model_block(model: &ModelUsage, total: u64, index: usize, theme: &Theme) -> [Line<'static>; 3] {
    let color = series_color(theme, index);
    let title = format!(
        "● {} ({})",
        display_model(&model.name),
        percent(model.total(), total)
    );
    let io = format!(
        "  {} {} · {} {}",
        i18n::tr("context.input", "输入"),
        format_tokens(model.input_tokens),
        i18n::tr("context.output", "输出"),
        format_tokens(model.output_tokens)
    );
    let cache = format!(
        "  {} {}",
        i18n::tr("context.cache_read", "缓存读取"),
        format_tokens(model.cached_tokens)
    );
    [
        Line::from(Span::styled(
            title,
            theme.fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(io, theme.dim())),
        Line::from(Span::styled(cache, theme.dim())),
    ]
}

fn percent(part: u64, total: u64) -> String {
    if total == 0 {
        return "0.0%".to_owned();
    }
    format!("{:.1}%", part as f64 * 100.0 / total as f64)
}

fn series_color(theme: &Theme, index: usize) -> Color {
    match index % 4 {
        0 => theme.rose,
        1 => theme.sage,
        2 => theme.amber,
        _ => theme.rust,
    }
}

fn range_line(selected: usize, theme: &Theme) -> (Line<'static>, [(usize, usize); 3]) {
    let labels = [
        i18n::tr("context.range.all", "全部"),
        i18n::tr("context.range.7", "近 7 天"),
        i18n::tr("context.range.30", "近 30 天"),
    ];
    let mut spans = Vec::new();
    let mut columns = [(0usize, 0usize); 3];
    let mut cursor = 0usize;
    for (index, label) in labels.iter().enumerate() {
        if index > 0 {
            let separator = " · ";
            spans.push(Span::styled(separator.to_owned(), theme.mute()));
            cursor += separator.width();
        }
        columns[index] = (cursor, label.width());
        let style = if index == selected {
            theme.fg(theme.rose).add_modifier(Modifier::BOLD)
        } else {
            theme.mute()
        };
        spans.push(Span::styled(label.clone(), style));
        cursor += label.width();
    }
    (Line::from(spans), columns)
}

fn stat_pair(
    left_label: &str,
    left_value: &str,
    right_label: &str,
    right_value: &str,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let half = (width / 2).max(1);
    let left = styled_stat(left_label, left_value, half, theme);
    let right = styled_stat(right_label, right_value, width.saturating_sub(half), theme);
    let used = line_width(&left);
    let gap = half.saturating_sub(used);
    let mut spans = left.spans;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right.spans);
    within(Line::from(spans), width)
}

fn styled_stat(label: &str, value: &str, width: usize, theme: &Theme) -> Line<'static> {
    let room = width.saturating_sub(label.width().saturating_add(2)).max(1);
    Line::from(vec![
        Span::styled(label.to_owned(), theme.fg(theme.text)),
        Span::raw("  "),
        Span::styled(
            fit(value, room),
            theme.fg(theme.rose).add_modifier(Modifier::BOLD),
        ),
    ])
}

fn token_breakdown(summary: &UsageSummary) -> String {
    format!(
        "{} {} · {} {} · {} {}",
        i18n::tr("context.input", "输入"),
        format_tokens(summary.input_tokens),
        i18n::tr("context.output", "输出"),
        format_tokens(summary.output_tokens),
        i18n::tr("context.cache_read", "缓存读取"),
        format_tokens(summary.cached_tokens)
    )
}

fn fun_line(duration: Duration) -> Option<String> {
    let seconds = duration.num_seconds().max(0);
    if seconds < 240 {
        return None;
    }
    let times = seconds / 240;
    Some(
        i18n::tr("context.fun_song", "最长会话约是一首 4 分钟歌曲的 {n} 倍")
            .replace("{n}", &times.to_string()),
    )
}

fn days_label(days: usize) -> String {
    i18n::tr("context.days", "{n} 天").replace("{n}", &days.to_string())
}

fn display_model(name: &str) -> String {
    if name.is_empty() {
        i18n::tr("context.unknown_model", "未知模型")
    } else {
        name.to_owned()
    }
}

fn format_day(date: NaiveDate) -> String {
    if locale_en() {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let index = usize::try_from(date.month()).unwrap_or(1).saturating_sub(1);
        format!("{} {}", MONTHS[index.min(11)], date.day())
    } else {
        format!("{}月{}日", date.month(), date.day())
    }
}

fn month_label(month: u32) -> String {
    if locale_en() {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let index = usize::try_from(month).unwrap_or(1).saturating_sub(1);
        MONTHS[index.min(11)].to_owned()
    } else {
        format!("{month}月")
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.num_seconds().max(0);
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let mins = (seconds % 3_600) / 60;
    if locale_en() {
        if days > 0 {
            format!("{days}d {hours}h {mins}m")
        } else if hours > 0 {
            format!("{hours}h {mins}m")
        } else {
            format!("{mins}m")
        }
    } else if days > 0 {
        format!("{days}天 {hours}小时 {mins}分")
    } else if hours > 0 {
        format!("{hours}小时 {mins}分")
    } else {
        format!("{mins}分")
    }
}

fn format_tokens(value: u64) -> String {
    if value < 1_000 {
        value.to_string()
    } else if value < 10_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else if value < 1_000_000 {
        format!("{}K", value / 1_000)
    } else if value < 10_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value < 1_000_000_000 {
        format!("{}M", value / 1_000_000)
    } else {
        format!("{:.1}B", value as f64 / 1_000_000_000.0)
    }
}

fn locale_en() -> bool {
    let locale = i18n::locale();
    locale.eq_ignore_ascii_case("en") || locale.starts_with("en-") || locale.starts_with("en_")
}

fn weekday_gutter(row: usize) -> String {
    let label = if locale_en() {
        match row {
            0 => "Mon",
            2 => "Wed",
            4 => "Fri",
            _ => "",
        }
    } else {
        match row {
            0 => "一",
            2 => "三",
            4 => "五",
            _ => "",
        }
    };
    gutter(label)
}

fn gutter(label: &str) -> String {
    let text = fit(label, 4);
    let pad = 4usize.saturating_sub(text.width());
    format!("{text}{}", " ".repeat(pad))
}

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    color: Color,
    skip: bool,
}

impl Cell {
    fn blank(color: Color) -> Self {
        Self {
            ch: ' ',
            color,
            skip: false,
        }
    }
}

fn blank_cells(count: usize, color: Color) -> Vec<Cell> {
    vec![Cell::blank(color); count]
}

fn write_text(cells: &mut [Cell], at: usize, text: &str, color: Color) -> bool {
    if at.saturating_add(text.width()) > cells.len() {
        return false;
    }
    let mut cursor = at;
    for ch in text.chars() {
        let cell_width = ch.width().unwrap_or(0).max(1);
        if cursor + cell_width > cells.len() {
            return false;
        }
        cells[cursor] = Cell {
            ch,
            color,
            skip: false,
        };
        for offset in 1..cell_width {
            cells[cursor + offset] = Cell {
                ch: ' ',
                color,
                skip: true,
            };
        }
        cursor += cell_width;
    }
    true
}

fn cells_line(prefix: &str, prefix_style: Style, cells: &[Cell]) -> Line<'static> {
    let mut spans = vec![Span::styled(prefix.to_owned(), prefix_style)];
    for cell in cells {
        if cell.skip {
            continue;
        }
        spans.push(Span::styled(
            cell.ch.to_string(),
            Style::default().fg(cell.color),
        ));
    }
    Line::from(spans)
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

fn within(line: Line<'static>, width: usize) -> Line<'static> {
    if width == 0 || line_width(&line) <= width {
        return line;
    }
    let mut spans = Vec::new();
    let mut used = 0usize;
    for span in line.spans {
        if used >= width {
            break;
        }
        let room = width - used;
        let text = span.content.into_owned();
        if text.width() <= room {
            used += text.width();
            spans.push(Span::styled(text, span.style));
        } else {
            spans.push(Span::styled(fit(&text, room), span.style));
            break;
        }
    }
    Line::from(spans)
}

fn fit(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let cell = ch.width().unwrap_or(0);
        if used + cell + 1 > max {
            break;
        }
        out.push(ch);
        used += cell;
    }
    if max > used {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_storage::UsageSample;
    use chrono::{TimeZone, Utc};

    fn day(year: i32, month: u32, dom: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, dom).unwrap()
    }

    fn at(date: NaiveDate, hour: u32) -> chrono::DateTime<Utc> {
        Utc.from_utc_datetime(&date.and_hms_opt(hour, 0, 0).unwrap())
    }

    fn sample(
        date: NaiveDate,
        model: &str,
        session: &str,
        input: u64,
        output: u64,
        cached: u64,
    ) -> UsageSample {
        UsageSample {
            day: date,
            model: model.to_owned(),
            session_id: session.to_owned(),
            input_tokens: input,
            output_tokens: output,
            cached_tokens: cached,
        }
    }

    fn activity() -> UsageActivity {
        UsageActivity {
            samples: vec![
                sample(day(2026, 8, 1), "Fable 5.1", "old", 1, 1, 0),
                sample(day(2026, 9, 14), "Fable 5.1", "live", 1000, 2000, 50),
                sample(day(2026, 9, 15), "Fable 5.1", "live", 10, 10, 0),
                sample(day(2026, 9, 16), "Opus 5.5", "live", 5, 5, 1),
                sample(day(2026, 9, 18), "Fable 5.1", "live", 100, 100, 0),
                sample(day(2026, 9, 19), "Fable 5.1", "live", 100, 100, 0),
                sample(day(2026, 9, 20), "Fable 5.1", "live", 100, 100, 0),
            ],
            sessions: vec![
                SessionSpan {
                    id: "old".to_owned(),
                    created_at: at(day(2026, 8, 1), 0),
                    updated_at: at(day(2026, 8, 1), 1),
                },
                SessionSpan {
                    id: "live".to_owned(),
                    created_at: at(day(2026, 9, 14), 0),
                    updated_at: at(day(2026, 9, 20), 12),
                },
            ],
        }
    }

    #[test]
    fn summary_filters_ranges_and_counts_streaks() {
        let today = day(2026, 9, 20);
        let all = summarize(&activity(), 0, today);
        assert_eq!(all.active_days, 7);
        assert_eq!(all.sessions, 2);
        assert_eq!(all.longest_streak, 3);
        assert_eq!(all.current_streak, 3);
        assert_eq!(all.most_active.unwrap().0, day(2026, 9, 14));
        assert_eq!(all.models[0].name, "Fable 5.1");
        assert_eq!(all.input_tokens, 1_316);
        assert!(all.longest_session.num_hours() >= 24 * 6);

        let week = summarize(&activity(), 1, today);
        assert_eq!(week.span_days, 7);
        assert_eq!(week.active_days, 6);
        assert_eq!(week.sessions, 1);
        assert_eq!(week.input_tokens, 1_315);
        assert_eq!(week.cached_tokens, 51);

        let month = summarize(&activity(), 2, today);
        assert_eq!(month.span_days, 30);
        assert_eq!(month.active_days, 6);
        assert!(month.models.iter().any(|model| model.name == "Opus 5.5"));
    }

    #[test]
    fn pages_name_models_and_fit_the_width() {
        let theme = Theme::dark();
        let summary = summarize(&activity(), 0, day(2026, 9, 20));
        for width in [24, 40, 80, 100] {
            let overview = overview(&summary, width, &theme);
            let models = models(&summary, width, &theme);
            let overview_text = text(&overview.lines);
            let model_text = text(&models.lines);
            assert!(overview_text.contains("常用模型"), "{overview_text}");
            assert!(overview_text.contains("Fable 5.1"), "{overview_text}");
            assert!(overview_text.contains("9月14日"), "{overview_text}");
            assert!(overview_text.contains('少'), "{overview_text}");
            assert!(overview_text.contains("4 分钟"), "{overview_text}");
            assert!(model_text.contains("每日 Token"), "{model_text}");
            assert!(model_text.contains("Opus 5.5"), "{model_text}");
            assert!(model_text.contains("99.7%"), "{model_text}");
            assert_eq!(overview.range_spans.len(), 3);
            assert!(overview.range_line.is_some());
            for line in overview.lines.iter().chain(models.lines.iter()) {
                assert!(
                    line_width(line) <= width,
                    "width {width} overflow {}",
                    line_width(line)
                );
            }
        }
    }

    fn text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
