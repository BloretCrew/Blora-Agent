// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Coral light/dark palettes from `@bloret-crew/blora-design` tokens.
//!
//! Hex values are copied from:
//! - `blora-design/packages/tokens/src/primitive/color.tokens.json` (light)
//! - `blora-design/packages/tokens/src/semantic/color-dark.tokens.json` (dark)
//! - semantic roles in `semantic/color-light.tokens.json`

use std::sync::Mutex;

use ratatui::style::{Color, Modifier, Style};

const fn hex(n: u32) -> Color {
    Color::Rgb(((n >> 16) & 0xff) as u8, ((n >> 8) & 0xff) as u8, (n & 0xff) as u8)
}

/// How the TUI chooses a palette. Session override wins over `BLORA_THEME`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePref {
    Auto,
    Dark,
    Light,
    Plain,
}

impl ThemePref {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Plain => "plain",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "dark" | "dusk" | "night" => Some(Self::Dark),
            "light" | "dawn" | "day" => Some(Self::Light),
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

/// Semantic colors for the TUI, mapped from Blora Design Coral tokens.
///
/// | field      | light token              | dark token               |
/// |------------|--------------------------|--------------------------|
/// | bg         | surface.canvas           | surface.canvas           |
/// | bg_raised  | surface.raised           | surface.raised           |
/// | bg_select  | surface.sunken           | surface.sunken           |
/// | text       | text.primary             | text.primary             |
/// | text_dim   | text.emphasis            | text.muted               |
/// | text_mute  | text.muted               | text.subtle              |
/// | rose       | action.primary.default   | action.primary.default   |
/// | sage       | status.success           | status.success           |
/// | amber      | status.warning           | status.warning           |
/// | rust       | status.danger            | status.danger            |
/// | hairline   | border.subtle            | border.subtle            |
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
    /// Coral dark scheme (`color-dark.tokens.json`).
    pub const fn dark() -> Self {
        Self {
            bg: hex(0x1716_1C),
            bg_raised: hex(0x2C29_31),
            bg_select: hex(0x3A36_3F),
            text: hex(0xF8F2_F4),
            text_dim: hex(0xA197_9D),
            text_mute: hex(0x786F_75),
            rose: hex(0xD18A_94),
            sage: hex(0x86A3_99),
            amber: hex(0xBBA2_7B),
            rust: hex(0xD277_79),
            hairline: hex(0x413C_41),
        }
    }

    /// Coral light scheme (`color.tokens.json` / `color-light.tokens.json`).
    pub const fn light() -> Self {
        Self {
            bg: hex(0xFAF7_F8),
            bg_raised: hex(0xF4EC_EE),
            bg_select: hex(0xE9DE_E1),
            text: hex(0x3031_43),
            text_dim: hex(0x5555_65),
            text_mute: hex(0x7471_7C),
            rose: hex(0x9F59_64),
            sage: hex(0x5B75_6B),
            amber: hex(0x806C_4F),
            rust: hex(0x9E55_59),
            hairline: hex(0xD9D2_D6),
        }
    }

    /// Alias kept for `/theme dusk`.
    pub const fn dusk() -> Self {
        Self::dark()
    }

    /// Alias kept for `/theme dawn`.
    pub const fn dawn() -> Self {
        Self::light()
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
            ThemePref::Dark => Self::dark(),
            ThemePref::Light => Self::light(),
            ThemePref::Auto => {
                if terminal_is_light() {
                    Self::light()
                } else {
                    Self::dark()
                }
            }
        }
    }

    /// OSC 12 sequence matching Coral primary, or reset under `NO_COLOR`.
    #[must_use]
    pub fn cursor_osc(self) -> &'static str {
        if self == Self::plain() {
            CURSOR_RESET
        } else if self == Self::light() {
            CURSOR_PRIMARY_LIGHT
        } else {
            CURSOR_PRIMARY_DARK
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

/// OSC 12 cursor = Coral `action.primary.default`. Reset with [`CURSOR_RESET`].
pub const CURSOR_PRIMARY_DARK: &str = "\x1b]12;#D18A94\x07";
pub const CURSOR_PRIMARY_LIGHT: &str = "\x1b]12;#9F5964\x07";
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
    fn dark_matches_coral_tokens() {
        let theme = Theme::dark();
        assert_eq!(theme.bg, hex(0x1716_1C));
        assert_eq!(theme.bg_raised, hex(0x2C29_31));
        assert_eq!(theme.rose, hex(0xD18A_94));
        assert_eq!(theme.sage, hex(0x86A3_99));
        assert_eq!(theme.rust, hex(0xD277_79));
        assert_eq!(theme.amber, hex(0xBBA2_7B));
        assert!(theme.is_dark());
        assert_eq!(Theme::dusk(), theme);
    }

    #[test]
    fn light_matches_coral_tokens() {
        let theme = Theme::light();
        assert_eq!(theme.bg, hex(0xFAF7_F8));
        assert_eq!(theme.bg_raised, hex(0xF4EC_EE));
        assert_eq!(theme.rose, hex(0x9F59_64));
        assert_eq!(theme.sage, hex(0x5B75_6B));
        assert_eq!(theme.rust, hex(0x9E55_59));
        assert_eq!(theme.amber, hex(0x806C_4F));
        assert!(!theme.is_dark());
        assert_eq!(Theme::dawn(), theme);
    }

    #[test]
    fn plain_uses_terminal_colors() {
        let theme = Theme::plain();
        assert_eq!(theme.bg, Color::Reset);
        assert_eq!(theme.rose, Color::Reset);
    }

    #[test]
    fn pref_parses_aliases() {
        assert_eq!(ThemePref::parse("dark"), Some(ThemePref::Dark));
        assert_eq!(ThemePref::parse("dusk"), Some(ThemePref::Dark));
        assert_eq!(ThemePref::parse("light"), Some(ThemePref::Light));
        assert_eq!(ThemePref::parse("dawn"), Some(ThemePref::Light));
        assert_eq!(ThemePref::parse("AUTO"), Some(ThemePref::Auto));
        assert_eq!(ThemePref::parse("nope"), None);
    }

    #[test]
    fn colorfgbg_15_is_light() {
        assert!(matches!("15", "7" | "15"));
        assert!(!matches!("0", "7" | "15"));
    }
}
