// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Terminal cell styling for the right panel.

use crate::theme::Theme;
use ratatui::style::Style;

pub(super) fn terminal_style(cell: &vt100::Cell, theme: &Theme) -> Style {
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
