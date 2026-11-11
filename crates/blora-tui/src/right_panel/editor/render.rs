// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Display-column clipping and overlays for cached highlight lines.

use ratatui::{
    style::Style,
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

use super::{document::TAB_WIDTH, highlight::CachedLine};

#[derive(Clone, Copy, Debug)]
pub(super) struct Overlay {
    pub(super) from: usize,
    pub(super) to: usize,
    pub(super) style: Style,
}

struct Cell {
    text: String,
    style: Style,
    column: usize,
    width: usize,
}

pub(super) fn render(
    line: &CachedLine,
    plain_style: Style,
    from_col: usize,
    width: usize,
    base: Style,
    overlays: &[Overlay],
) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    let visible_end = from_col.saturating_add(width);
    let mut cells = Vec::new();
    let mut column = 0;
    for segment in &line.segments {
        let style = if line.plain {
            plain_style
        } else {
            plain_style.patch(segment.style)
        };
        for ch in segment.text.chars() {
            if column >= visible_end {
                break;
            }
            if ch == '\t' {
                let tab = TAB_WIDTH - column % TAB_WIDTH;
                let start = column.max(from_col);
                let end = (column + tab).min(visible_end);
                if end > start {
                    cells.push(Cell {
                        text: " ".repeat(end - start),
                        style,
                        column: start,
                        width: end - start,
                    });
                }
                column += tab;
                continue;
            }
            let char_width = ch.width().unwrap_or(0);
            if char_width == 0 {
                if column >= from_col && column < visible_end {
                    cells.push(Cell {
                        text: ch.to_string(),
                        style,
                        column,
                        width: 0,
                    });
                }
                continue;
            }
            if column >= from_col && column + char_width <= visible_end {
                cells.push(Cell {
                    text: ch.to_string(),
                    style,
                    column,
                    width: char_width,
                });
            }
            column += char_width;
        }
        if column >= visible_end {
            break;
        }
    }
    let mut visible = Vec::with_capacity(cells.len());
    for mut cell in cells {
        if base != Style::default() {
            cell.style = cell.style.patch(base);
        }
        for overlay in overlays {
            if cell.column < overlay.to && cell.column + cell.width.max(1) > overlay.from {
                cell.style = cell.style.patch(overlay.style);
            }
        }
        visible.push(cell);
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut text = String::new();
    let mut current = None;
    for cell in visible {
        if current != Some(cell.style) {
            if let Some(style) = current {
                spans.push(Span::styled(std::mem::take(&mut text), style));
            }
            current = Some(cell.style);
        }
        text.push_str(&cell.text);
    }
    if let Some(style) = current {
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::right_panel::editor::highlight::Segment;

    fn cached(text: &str) -> CachedLine {
        CachedLine {
            segments: vec![Segment {
                style: Style::default(),
                text: text.to_owned(),
            }],
            plain: false,
        }
    }

    #[test]
    fn clips_horizontal_scroll_without_splitting_wide_characters() {
        let line = cached("甲🙂abc");
        let rendered = render(&line, Style::default(), 2, 3, Style::default(), &[]);
        assert_eq!(rendered.to_string(), "🙂a");
        let rendered = render(&line, Style::default(), 4, 3, Style::default(), &[]);
        assert_eq!(rendered.to_string(), "abc");
    }

    #[test]
    fn expands_tabs_at_tab_stops() {
        let line = cached("a\tb");
        let rendered = render(&line, Style::default(), 0, 8, Style::default(), &[]);
        assert_eq!(rendered.to_string(), "a   b");
    }

    #[test]
    fn overlays_apply_to_covered_cells() {
        let line = cached("abcdef");
        let overlay = Overlay {
            from: 2,
            to: 4,
            style: Style::default().bg(ratatui::style::Color::Red),
        };
        let rendered = render(&line, Style::default(), 0, 6, Style::default(), &[overlay]);
        let text: String = rendered
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(text, "abcdef");
        assert!(rendered.spans.len() >= 3);
    }
}
