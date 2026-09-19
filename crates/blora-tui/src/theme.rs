// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Warm dusk / dawn palettes. Rose and sage follow Blora Design; the rest is TUI-only.

use std::sync::Mutex;

use ratatui::style::{Color, Modifier, Style};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// How the TUI chooses a palette. Session override wins over `BLORA_THEME`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePref {
    Auto,
    Dusk,
    Dawn,
    Plain,
}

impl ThemePref {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dusk => "dusk",
            Self::Dawn => "dawn",
            Self::Plain => "plain",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "dusk" | "dark" | "night" => Some(Self::Dusk),
            "dawn" | "light" | "day" => Some(Self::Dawn),
            "plain" | "none" | "off" => Some(Self::Plain),
            _ => None,
        }
    }
}

static PREF: Mutex<Option<ThemePref>> = Mutex::new(None);

/// Set the session theme preference. `None` falls back to `BLORA_THEME` / auto.
pub fn set_pref(pref: ThemePref) {
    if let Ok(mut slot) = PREF.lock() {
        *slot = Some(pref);
    }
}

#[must_use]
pub fn pref() -> ThemePref {
    if let Ok(slot) = PREF.lock() {
        if let Some(pref) = *slot {
            return pref;
        }
    }
    std::env::var("BLORA_THEME")
        .ok()
        .and_then(|value| ThemePref::parse(&value))
        .unwrap_or(ThemePref::Auto)
}

/// Semantic colors for the TUI. `dusk` is the dark default; `dawn` is light.
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

    /// Warm paper field for light terminals. Raised panels stay light so the
    /// slash menu does not punch a dark hole into a pale background.
    pub const fn dawn() -> Self {
        Self {
            bg: rgb(250, 247, 244),
            bg_raised: rgb(236, 230, 226),
            bg_select: rgb(214, 200, 196),
            text: rgb(42, 34, 36),
            text_dim: rgb(92, 78, 82),
            text_mute: rgb(132, 116, 120),
            rose: rgb(168, 78, 92),
            sage: rgb(62, 118, 98),
            amber: rgb(158, 102, 48),
            rust: rgb(176, 64, 64),
            hairline: rgb(214, 204, 200),
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
    pub fn is_dark(self) -> bool {
        self == Self::dusk()
    }

    #[must_use]
    pub fn current() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return Self::plain();
        }
        match pref() {
            ThemePref::Plain => Self::plain(),
            ThemePref::Dusk => Self::dusk(),
            ThemePref::Dawn => Self::dawn(),
            ThemePref::Auto => {
                if terminal_is_light() {
                    Self::dawn()
                } else {
                    Self::dusk()
                }
            }
        }
    }

    /// OSC 12 sequence matching the active rose, or reset under `NO_COLOR`.
    #[must_use]
    pub fn cursor_osc(self) -> &'static str {
        if self == Self::plain() {
            CURSOR_RESET
        } else if self == Self::dawn() {
            CURSOR_ROSE_DAWN
        } else {
            CURSOR_ROSE
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
pub const CURSOR_ROSE_DAWN: &str = "\x1b]12;#a84e5c\x07";
pub const CURSOR_RESET: &str = "\x1b]112\x07";

/// `COLORFGBG` is `fg;bg` with ANSI color indexes. 7 and 15 are white-ish.
fn terminal_is_light() -> bool {
    let Ok(value) = std::env::var("COLORFGBG") else {
        return false;
    };
    let Some(bg) = value.rsplit(';').next() else {
        return false;
    };
    matches!(bg.trim(), "7" | "15")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dusk_is_truecolor() {
        let theme = Theme::dusk();
        assert!(matches!(theme.bg, Color::Rgb(_, _, _)));
        assert_ne!(theme.rose, theme.sage);
        assert_ne!(theme.bg, theme.text);
        assert!(theme.is_dark());
    }

    #[test]
    fn dawn_contrasts_on_paper() {
        let theme = Theme::dawn();
        assert_ne!(theme.bg, theme.text);
        assert_ne!(theme.bg_raised, theme.text);
        assert_ne!(theme.rose, theme.sage);
        assert!(!theme.is_dark());
    }

    #[test]
    fn plain_uses_terminal_colors() {
        let theme = Theme::plain();
        assert_eq!(theme.bg, Color::Reset);
        assert_eq!(theme.rose, Color::Reset);
    }

    #[test]
    fn pref_parses_aliases() {
        assert_eq!(ThemePref::parse("dark"), Some(ThemePref::Dusk));
        assert_eq!(ThemePref::parse("light"), Some(ThemePref::Dawn));
        assert_eq!(ThemePref::parse("AUTO"), Some(ThemePref::Auto));
        assert_eq!(ThemePref::parse("nope"), None);
    }

    #[test]
    fn colorfgbg_15_is_light() {
        assert!(matches!("15", "7" | "15"));
        assert!(!matches!("0", "7" | "15"));
    }
}
