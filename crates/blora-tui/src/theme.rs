// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Warm dusk palette. Rose and sage follow Blora Design; the rest is TUI-only.

use ratatui::style::{Color, Modifier, Style};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// Semantic colors for the TUI. `dusk` is the default; `plain` honors `NO_COLOR`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub bg: Color,
    pub bg_raised: Color,
    pub bg_select: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_mute: Color,
    pub rose: Color,
    pub sage: Color,
    pub amber: Color,
    pub rust: Color,
    pub hairline: Color,
}

impl Theme {
    /// Warm near-black field, rose for the human, sage for the agent.
    pub const fn dusk() -> Self {
        Self {
            bg: rgb(18, 15, 16),
            bg_raised: rgb(32, 26, 28),
            bg_select: rgb(48, 38, 42),
            text: rgb(244, 238, 239),
            text_dim: rgb(186, 168, 172),
            text_mute: rgb(122, 106, 110),
            rose: rgb(201, 123, 134),
            sage: rgb(138, 171, 156),
            amber: rgb(212, 165, 116),
            rust: rgb(196, 92, 92),
            hairline: rgb(56, 46, 50),
        }
    }

    pub const fn plain() -> Self {
        Self {
            bg: Color::Reset,
            bg_raised: Color::Reset,
            bg_select: Color::Reset,
            text: Color::Reset,
            text_dim: Color::Reset,
            text_mute: Color::Reset,
            rose: Color::Reset,
            sage: Color::Reset,
            amber: Color::Reset,
            rust: Color::Reset,
            hairline: Color::Reset,
        }
    }

    #[must_use]
    pub fn current() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            Self::plain()
        } else {
            Self::dusk()
        }
    }

    #[must_use]
    pub fn fg(self, color: Color) -> Style {
        Style::default().fg(color)
    }

    #[must_use]
    pub fn base(self) -> Style {
        self.fg(self.text)
    }

    #[must_use]
    pub fn dim(self) -> Style {
        self.fg(self.text_dim)
    }

    #[must_use]
    pub fn mute(self) -> Style {
        self.fg(self.text_mute)
    }

    #[must_use]
    pub fn rose_bold(self) -> Style {
        self.fg(self.rose).add_modifier(Modifier::BOLD)
    }

    #[must_use]
    pub fn sage_bold(self) -> Style {
        self.fg(self.sage).add_modifier(Modifier::BOLD)
    }
}

/// OSC 12 cursor color matching dusk rose. Reset with [`CURSOR_RESET`].
pub const CURSOR_ROSE: &str = "\x1b]12;#c97b86\x07";
pub const CURSOR_RESET: &str = "\x1b]112\x07";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dusk_is_truecolor() {
        let theme = Theme::dusk();
        assert!(matches!(theme.bg, Color::Rgb(_, _, _)));
        assert_ne!(theme.rose, theme.sage);
        assert_ne!(theme.bg, theme.text);
    }

    #[test]
    fn plain_uses_terminal_colors() {
        let theme = Theme::plain();
        assert_eq!(theme.bg, Color::Reset);
        assert_eq!(theme.rose, Color::Reset);
    }
}
