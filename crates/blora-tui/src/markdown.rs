// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Lightweight Markdown → ratatui lines for the transcript.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Theme;

/// Render Markdown into wrapped transcript body lines (already indented).
#[must_use]
pub fn render(text: &str, width: usize, needle: Option<&str>, theme: &Theme) -> Vec<Line<'static>> {
    let width = width.max(8);
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut fence_buf: Vec<String> = Vec::new();
    let mut table_buf: Vec<String> = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();

    let flush_para = |out: &mut Vec<Line<'static>>, paragraph: &mut Vec<String>| {
        if paragraph.is_empty() {
            return;
        }
        let joined = paragraph.join("\n");
        paragraph.clear();
        out.extend(render_block(&joined, width, needle, theme));
    };

    for raw in text.split('\n') {
        let trimmed = raw.trim_end();
        if let Some(rest) = fence_marker(trimmed) {
            if in_fence {
                out.extend(render_code_block(&fence_buf, width, needle, theme));
                fence_buf.clear();
                in_fence = false;
            } else {
                flush_para(&mut out, &mut paragraph);
                let _lang = rest.trim();
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            fence_buf.push(raw.to_owned());
            continue;
        }
        if is_table_line(trimmed) {
            flush_para(&mut out, &mut paragraph);
            table_buf.push(trimmed.to_owned());
            continue;
        }
        if !table_buf.is_empty() {
            out.extend(flush_table(&mut table_buf, width, needle, theme));
        }
        if trimmed.trim().is_empty() {
            flush_para(&mut out, &mut paragraph);
            continue;
        }
        if heading_level(trimmed).is_some()
            || is_hr(trimmed)
            || list_prefix(trimmed).is_some()
            || trimmed.starts_with('>')
        {
            flush_para(&mut out, &mut paragraph);
            out.extend(render_block(trimmed, width, needle, theme));
            continue;
        }
        paragraph.push(trimmed.to_owned());
    }
    if in_fence {
        out.extend(render_code_block(&fence_buf, width, needle, theme));
    }
    if !table_buf.is_empty() {
        out.extend(flush_table(&mut table_buf, width, needle, theme));
    }
    flush_para(&mut out, &mut paragraph);
    if out.is_empty() {
        out.push(indent_line(Vec::new(), theme));
    }
    out
}

fn fence_marker(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if t.starts_with("```") {
        Some(&t[3..])
    } else if t.starts_with("~~~") {
        Some(&t[3..])
    } else {
        None
    }
}

fn heading_level(line: &str) -> Option<(u8, &str)> {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    t.get(hashes..)
        .and_then(|rest| rest.strip_prefix(' '))
        .map(|rest| (hashes as u8, rest.trim()))
}

fn is_table_line(line: &str) -> bool {
    let t = line.trim();
    t.contains('|') && !t.starts_with("```")
}

fn is_table_sep_row(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let t = cell.trim();
            let inner = t.trim_matches(':').trim();
            !inner.is_empty() && inner.chars().all(|ch| ch == '-')
        })
}

fn split_table_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|cell| cell.trim().to_owned()).collect()
}

fn flush_table(
    rows: &mut Vec<String>,
    width: usize,
    needle: Option<&str>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let raw = std::mem::take(rows);
    if raw.len() < 2 || !raw.iter().any(|line| is_table_sep_row(&split_table_row(line))) {
        return raw
            .into_iter()
            .flat_map(|line| render_block(&line, width, needle, theme))
            .collect();
    }
    render_table(&raw, width, needle, theme)
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

fn parse_align(cell: &str) -> Align {
    let t = cell.trim();
    match (t.starts_with(':'), t.ends_with(':')) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    }
}

fn pad_cell(text: &str, width: usize, align: Align) -> String {
    let w = text.width();
    if w >= width {
        return truncate_width(text, width);
    }
    let pad = width - w;
    match align {
        Align::Left => format!("{text}{}", " ".repeat(pad)),
        Align::Right => format!("{}{text}", " ".repeat(pad)),
        Align::Center => {
            let left = pad / 2;
            format!("{}{text}{}", " ".repeat(left), " ".repeat(pad - left))
        }
    }
}

fn truncate_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let cw = ch.width().unwrap_or(0);
        if used + cw + 1 > width {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out.push('…');
    let extra = width.saturating_sub(out.width());
    if extra > 0 {
        out.push_str(&" ".repeat(extra));
    }
    out
}

fn render_table(
    lines: &[String],
    width: usize,
    needle: Option<&str>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let parsed: Vec<Vec<String>> = lines.iter().map(|line| split_table_row(line)).collect();
    let sep_at = parsed.iter().position(|row| is_table_sep_row(row));
    let aligns = sep_at
        .map(|index| {
            parsed[index]
                .iter()
                .map(|cell| parse_align(cell))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut body: Vec<(bool, Vec<String>)> = Vec::new();
    for (index, row) in parsed.into_iter().enumerate() {
        if sep_at == Some(index) {
            continue;
        }
        body.push((sep_at.is_some_and(|sep| index < sep), row));
    }
    if body.is_empty() {
        return Vec::new();
    }
    let cols = body.iter().map(|(_, row)| row.len()).max().unwrap_or(0);
    if cols == 0 {
        return Vec::new();
    }
    let mut col_w = vec![0usize; cols];
    for (_, row) in &body {
        for (index, cell) in row.iter().enumerate() {
            col_w[index] = col_w[index].max(cell.width());
        }
    }
    let sep_w = 3; // ` │ `
    let usable = width.saturating_sub(2).max(cols);
    let min_total = cols + sep_w * cols.saturating_sub(1);
    if col_w.iter().sum::<usize>() + sep_w * cols.saturating_sub(1) > usable {
        let extra = usable.saturating_sub(min_total);
        let base = extra / cols;
        let mut rem = extra % cols;
        for width in &mut col_w {
            *width = 1 + base + usize::from(rem > 0);
            rem = rem.saturating_sub(1);
        }
    }
    let mut out = Vec::new();
    for (is_header, row) in body {
        let mut line = String::new();
        for index in 0..cols {
            if index > 0 {
                line.push_str(" │ ");
            }
            let cell = row.get(index).map(String::as_str).unwrap_or("");
            let align = aligns.get(index).copied().unwrap_or(Align::Left);
            line.push_str(&pad_cell(cell, col_w[index], align));
        }
        let style = if is_header {
            theme.fg(theme.text).add_modifier(Modifier::BOLD)
        } else {
            theme.fg(theme.text)
        };
        let mut spans = highlight(line, style, needle, theme);
        if is_header {
            spans = spans
                .into_iter()
                .map(|span| Span::styled(span.content, span.style.add_modifier(Modifier::BOLD)))
                .collect();
        }
        out.push(indent_line(spans, theme));
        if is_header {
            let mut rule = String::new();
            for index in 0..cols {
                if index > 0 {
                    rule.push_str("─┼─");
                }
                rule.push_str(&"─".repeat(col_w[index].max(1)));
            }
            out.push(indent_line(
                vec![Span::styled(rule, theme.mute())],
                theme,
            ));
        }
    }
    out
}

fn is_hr(line: &str) -> bool {
    let t: String = line.chars().filter(|ch| !ch.is_whitespace()).collect();
    t.len() >= 3 && t.chars().all(|ch| ch == '-' || ch == '*' || ch == '_')
}

fn list_prefix(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(marker) {
            return Some(("• ", rest));
        }
    }
    let digits = t.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits > 0 {
        let rest = &t[digits..];
        if let Some(body) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            let n = &t[..digits];
            return Some((n, body));
        }
    }
    None
}

fn render_block(text: &str, width: usize, needle: Option<&str>, theme: &Theme) -> Vec<Line<'static>> {
    if let Some((level, title)) = heading_level(text) {
        let style = match level {
            1 => theme.fg(theme.rose).add_modifier(Modifier::BOLD),
            2 => theme.fg(theme.sage).add_modifier(Modifier::BOLD),
            _ => theme.fg(theme.text).add_modifier(Modifier::BOLD),
        };
        return wrap_spans(render_inlines(title, style, needle, theme), width, theme)
            .into_iter()
            .map(|spans| indent_line(spans, theme))
            .collect();
    }
    if is_hr(text) {
        let rule = "─".repeat(width.saturating_sub(2).max(3));
        return vec![indent_line(
            vec![Span::styled(rule, theme.mute())],
            theme,
        )];
    }
    if let Some(rest) = text.trim_start().strip_prefix('>') {
        let body = rest.strip_prefix(' ').unwrap_or(rest);
        return wrap_spans(
            render_inlines(
                body,
                theme.fg(theme.text_dim).add_modifier(Modifier::ITALIC),
                needle,
                theme,
            ),
            width.saturating_sub(2),
            theme,
        )
        .into_iter()
        .map(|row| {
            let mut spans = vec![Span::styled("│ ".to_owned(), theme.mute())];
            spans.extend(row);
            indent_line(spans, theme)
        })
        .collect();
    }
    if let Some((marker, body)) = list_prefix(text) {
        let marker = if marker == "• " {
            "• ".to_owned()
        } else {
            format!("{marker}. ")
        };
        let marker_w = marker.width();
        let inner_w = width.saturating_sub(marker_w).max(8);
        let rows = wrap_spans(
            render_inlines(body, theme.fg(theme.text), needle, theme),
            inner_w,
            theme,
        );
        return rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let lead = if index == 0 {
                    marker.clone()
                } else {
                    " ".repeat(marker_w)
                };
                let mut spans = vec![Span::styled(lead, theme.fg(theme.text_dim))];
                spans.extend(row);
                indent_line(spans, theme)
            })
            .collect();
    }
    wrap_spans(
        render_inlines(text, theme.fg(theme.text), needle, theme),
        width,
        theme,
    )
    .into_iter()
    .map(|spans| indent_line(spans, theme))
    .collect()
}

fn render_code_block(
    lines: &[String],
    width: usize,
    needle: Option<&str>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(2).max(4);
    let style = theme.fg(theme.sage);
    let mut out = Vec::new();
    if lines.is_empty() {
        out.push(indent_line(
            vec![Span::styled("│ ".to_owned(), theme.mute())],
            theme,
        ));
        return out;
    }
    for line in lines {
        let shown = if line.width() > inner {
            crate::view::wrap_text(line, inner)
        } else {
            vec![line.clone()]
        };
        for piece in shown {
            let mut spans = vec![Span::styled("│ ".to_owned(), theme.mute())];
            spans.extend(highlight(piece, style, needle, theme));
            out.push(indent_line(spans, theme));
        }
    }
    out
}

fn render_inlines(
    text: &str,
    base: Style,
    needle: Option<&str>,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '`' {
            if let Some(end) = find_closing(&chars, i + 1, '`') {
                let inner: String = chars[i + 1..end].iter().collect();
                out.extend(highlight(
                    inner,
                    theme.fg(theme.amber),
                    needle,
                    theme,
                ));
                i = end + 1;
                continue;
            }
        }
        if take_delim(&chars, i, "**") || take_delim(&chars, i, "__") {
            let delim = if take_delim(&chars, i, "**") { "**" } else { "__" };
            if let Some(end) = find_delim(&chars, i + 2, delim) {
                let inner: String = chars[i + 2..end].iter().collect();
                out.extend(highlight(
                    inner,
                    base.add_modifier(Modifier::BOLD),
                    needle,
                    theme,
                ));
                i = end + delim.len();
                continue;
            }
        }
        if take_delim(&chars, i, "~~") {
            if let Some(end) = find_delim(&chars, i + 2, "~~") {
                let inner: String = chars[i + 2..end].iter().collect();
                out.extend(highlight(
                    inner,
                    base.add_modifier(Modifier::CROSSED_OUT),
                    needle,
                    theme,
                ));
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '*' || (chars[i] == '_' && italic_ok(&chars, i)) {
            let mark = chars[i];
            if let Some(end) = find_char(&chars, i + 1, mark) {
                let inner: String = chars[i + 1..end].iter().collect();
                out.extend(highlight(
                    inner,
                    base.add_modifier(Modifier::ITALIC),
                    needle,
                    theme,
                ));
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some((label, href, next)) = parse_link(&chars, i) {
                out.extend(highlight(
                    label,
                    theme.fg(theme.sage).add_modifier(Modifier::UNDERLINED),
                    needle,
                    theme,
                ));
                if !href.is_empty() {
                    out.push(Span::styled(format!(" ({href})"), theme.mute()));
                }
                i = next;
                continue;
            }
        }
        let start = i;
        i += 1;
        while i < chars.len()
            && chars[i] != '`'
            && chars[i] != '*'
            && chars[i] != '_'
            && chars[i] != '~'
            && chars[i] != '['
        {
            i += 1;
        }
        let chunk: String = chars[start..i].iter().collect();
        out.extend(highlight(chunk, base, needle, theme));
    }
    out
}

fn italic_ok(chars: &[char], i: usize) -> bool {
    let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
    !prev.is_some_and(|ch| ch.is_ascii_alphanumeric())
}

fn take_delim(chars: &[char], i: usize, delim: &str) -> bool {
    let d: Vec<char> = delim.chars().collect();
    chars.get(i..i + d.len()) == Some(d.as_slice())
}

fn find_delim(chars: &[char], start: usize, delim: &str) -> Option<usize> {
    let d: Vec<char> = delim.chars().collect();
    let mut i = start;
    while i + d.len() <= chars.len() {
        if chars[i..i + d.len()] == d[..] {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_closing(chars: &[char], start: usize, mark: char) -> Option<usize> {
    chars[start..].iter().position(|ch| *ch == mark).map(|p| start + p)
}

fn find_char(chars: &[char], start: usize, mark: char) -> Option<usize> {
    find_closing(chars, start, mark).filter(|end| *end > start)
}

fn parse_link(chars: &[char], i: usize) -> Option<(String, String, usize)> {
    let close = find_char(chars, i + 1, ']')?;
    if chars.get(close + 1).copied() != Some('(') {
        return None;
    }
    let end = find_char(chars, close + 2, ')')?;
    let label: String = chars[i + 1..close].iter().collect();
    let href: String = chars[close + 2..end].iter().collect();
    Some((label, href, end + 1))
}

fn highlight(
    text: String,
    style: Style,
    needle: Option<&str>,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let Some(needle) = needle.filter(|item| !item.is_empty()) else {
        return vec![Span::styled(text, style)];
    };
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut rest = text.as_str();
    let mut rest_lower = lower.as_str();
    let hit = theme.fg(theme.amber).add_modifier(Modifier::BOLD);
    while let Some(at) = rest_lower.find(needle) {
        if at > 0 {
            spans.push(Span::styled(rest[..at].to_owned(), style));
        }
        let end = at + needle.len();
        spans.push(Span::styled(rest[at..end].to_owned(), hit));
        rest = &rest[end..];
        rest_lower = &rest_lower[end..];
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_owned(), style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(text, style));
    }
    spans
}

fn wrap_spans(spans: Vec<Span<'static>>, width: usize, _theme: &Theme) -> Vec<Vec<Span<'static>>> {
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut w = 0usize;
    for span in spans {
        let style = span.style;
        for ch in span.content.chars() {
            let cw = ch.width().unwrap_or(0);
            if w + cw > width && w > 0 {
                lines.push(Vec::new());
                w = 0;
            }
            let line = lines.last_mut().expect("line");
            if let Some(last) = line.last_mut() {
                if last.style == style {
                    last.content.to_mut().push(ch);
                    w += cw;
                    continue;
                }
            }
            line.push(Span::styled(ch.to_string(), style));
            w += cw;
        }
    }
    lines
}

fn indent_line(spans: Vec<Span<'static>>, theme: &Theme) -> Line<'static> {
    let mut all = vec![Span::styled("  ", theme.fg(theme.text))];
    all.extend(spans);
    Line::from(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn theme() -> Theme {
        Theme::current()
    }

    fn flat(lines: &[Line]) -> String {
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

    #[test]
    fn renders_heading_bold_code_and_list() {
        let md = "# Hello\n\nThis is **bold** and `code`.\n\n- one\n- two\n";
        let lines = render(md, 40, None, &theme());
        let text = flat(&lines);
        assert!(text.contains("Hello"), "{text}");
        assert!(text.contains("bold"), "{text}");
        assert!(text.contains("code"), "{text}");
        assert!(text.contains("• "), "{text}");
        let hello = lines
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("Hello"))
            })
            .expect("heading line");
        assert!(hello
            .spans
            .iter()
            .any(|span| span.style.add_modifier.contains(Modifier::BOLD)));
    }

    #[test]
    fn renders_fenced_code() {
        let md = "```rs\nfn main() {}\n```";
        let lines = render(md, 40, None, &theme());
        let text = flat(&lines);
        assert!(text.contains("fn main()"), "{text}");
        assert!(text.contains("│ "), "{text}");
    }

    #[test]
    fn renders_pipe_table_with_aligned_columns() {
        let md = "| Name | Qty |\n| --- | ---: |\n| apples | 12 |\n| tea | 3 |\n";
        let lines = render(md, 40, None, &theme());
        let text = flat(&lines);
        assert!(text.contains("Name"), "{text}");
        assert!(text.contains("apples"), "{text}");
        assert!(text.contains("│"), "{text}");
        assert!(text.contains("┼"), "{text}");
        assert!(
            !text.contains("| ---"),
            "separator row should not be shown raw: {text}"
        );
    }
}
