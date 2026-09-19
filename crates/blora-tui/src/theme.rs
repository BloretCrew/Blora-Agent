// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Palettes copied from `@bloret-crew/blora-design` theme tokens.
//!
//! Hex values come from `blora-design/packages/tokens/src/themes/*.tokens.json`.
//! Names and blurbs match `packages/blora-design/src/locales/zh-CN.ts`.

use std::sync::Mutex;

use ratatui::style::{Color, Modifier, Style};

const fn hex(n: u32) -> Color {
    Color::Rgb(
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
    )
}

/// Blora Design `data-blora-theme` presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    Coral,
    Indigo,
    Graphite,
    Mono,
    Circuit,
    Dusk,
}

impl Palette {
    pub const ALL: [Self; 6] = [
        Self::Coral,
        Self::Indigo,
        Self::Graphite,
        Self::Mono,
        Self::Circuit,
        Self::Dusk,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Coral => "coral",
            Self::Indigo => "indigo",
            Self::Graphite => "graphite",
            Self::Mono => "mono",
            Self::Circuit => "circuit",
            Self::Dusk => "dusk",
        }
    }

    #[must_use]
    pub fn name_zh(self) -> &'static str {
        match self {
            Self::Coral => "珊瑚",
            Self::Indigo => "靛蓝",
            Self::Graphite => "石墨",
            Self::Mono => "单色",
            Self::Circuit => "电路",
            Self::Dusk => "暮色",
        }
    }

    #[must_use]
    pub fn about_zh(self) -> &'static str {
        match self {
            Self::Coral => "深靛灰与柔和珊瑚红",
            Self::Indigo => "冷灰基底与沉静蓝",
            Self::Graphite => "冷灰界面与低饱和钢蓝",
            Self::Mono => "纯中性灰与近黑主色",
            Self::Circuit => "碳灰界面与克制青色",
            Self::Dusk => "暮色灰紫",
        }
    }

    /// Card swatches from the theming add-on's `THEME_PRESETS`.
    #[must_use]
    pub fn swatches(self) -> [Color; 5] {
        match self {
            Self::Coral => [
                hex(0xFAF7_F8),
                hex(0x3031_43),
                hex(0x9F59_64),
                hex(0x5D66_80),
                hex(0x5B75_6B),
            ],
            Self::Indigo => [
                hex(0xF4F5_F8),
                hex(0x405D_87),
                hex(0x5575_6F),
                hex(0xA74B_52),
                hex(0xAF8A_55),
            ],
            Self::Graphite => [
                hex(0xF6F7_F8),
                hex(0x171A_1F),
                hex(0x4F65_78),
                hex(0x596A_86),
                hex(0x5B75_6B),
            ],
            Self::Mono => [
                hex(0xFAFA_F9),
                hex(0x1111_10),
                hex(0x3436_3A),
                hex(0x5E66_72),
                hex(0x616D_67),
            ],
            Self::Circuit => [
                hex(0xF4F5_F5),
                hex(0x161A_1A),
                hex(0x3E6C_70),
                hex(0x536D_7D),
                hex(0x4F73_68),
            ],
            Self::Dusk => [
                hex(0xF6F4_F8),
                hex(0x3A35_48),
                hex(0x7A6B_8A),
                hex(0x5A6B_7A),
                hex(0x8A7A_6A),
            ],
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "coral" | "珊瑚" => Some(Self::Coral),
            "indigo" | "靛蓝" => Some(Self::Indigo),
            "graphite" | "石墨" => Some(Self::Graphite),
            "mono" | "单色" => Some(Self::Mono),
            "circuit" | "电路" => Some(Self::Circuit),
            "dusk" | "暮色" => Some(Self::Dusk),
            _ => None,
        }
    }
}

/// Light / dark / auto / plain, matching `data-blora-color-scheme`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Auto,
    Dark,
    Light,
    Plain,
}

impl Scheme {
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
            "dark" | "night" => Some(Self::Dark),
            "light" | "dawn" | "day" => Some(Self::Light),
            "plain" | "none" | "off" => Some(Self::Plain),
            _ => None,
        }
    }
}

/// Combined preference. `parse` accepts a palette, a scheme, or `palette-scheme`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePref {
    pub palette: Palette,
    pub scheme: Scheme,
}

impl ThemePref {
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim().to_ascii_lowercase();
        if let Some((left, right)) = raw.split_once('-') {
            return Some(Self {
                palette: Palette::parse(left)?,
                scheme: Scheme::parse(right)?,
            });
        }
        if let Some(palette) = Palette::parse(&raw) {
            return Some(Self {
                palette,
                scheme: scheme(),
            });
        }
        if let Some(scheme) = Scheme::parse(&raw) {
            return Some(Self {
                palette: palette(),
                scheme,
            });
        }
        None
    }
}

#[derive(Clone, Copy, Debug)]
struct ThemeState {
    palette: Palette,
    scheme: Scheme,
}

static STATE: Mutex<Option<ThemeState>> = Mutex::new(None);

pub fn set_pref(pref: ThemePref) {
    if let Ok(mut slot) = STATE.lock() {
        *slot = Some(ThemeState {
            palette: pref.palette,
            scheme: pref.scheme,
        });
    }
}

#[must_use]
pub fn palette() -> Palette {
    if let Ok(slot) = STATE.lock() {
        if let Some(state) = *slot {
            return state.palette;
        }
    }
    std::env::var("BLORA_THEME")
        .ok()
        .and_then(|value| {
            Palette::parse(value.split('-').next().unwrap_or(&value))
        })
        .unwrap_or(Palette::Coral)
}

#[must_use]
pub fn scheme() -> Scheme {
    if let Ok(slot) = STATE.lock() {
        if let Some(state) = *slot {
            return state.scheme;
        }
    }
    if let Some(scheme) = std::env::var("BLORA_COLOR_SCHEME")
        .ok()
        .and_then(|value| Scheme::parse(&value))
    {
        return scheme;
    }
    std::env::var("BLORA_THEME")
        .ok()
        .and_then(|value| {
            value
                .split_once('-')
                .and_then(|(_, scheme)| Scheme::parse(scheme))
                .or_else(|| Scheme::parse(&value))
        })
        .unwrap_or(Scheme::Auto)
}

/// Semantic colors for the TUI, mapped from Blora Design tokens.
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
    dark: bool,
}

const fn tokens(
    bg: u32,
    raised: u32,
    select: u32,
    text: u32,
    dim: u32,
    mute: u32,
    rose: u32,
    sage: u32,
    amber: u32,
    rust: u32,
    hairline: u32,
    dark: bool,
) -> Theme {
    Theme {
        bg: hex(bg),
        bg_raised: hex(raised),
        bg_select: hex(select),
        text: hex(text),
        text_dim: hex(dim),
        text_mute: hex(mute),
        rose: hex(rose),
        sage: hex(sage),
        amber: hex(amber),
        rust: hex(rust),
        hairline: hex(hairline),
        dark,
    }
}

impl Theme {
    #[must_use]
    pub const fn of(palette: Palette, dark: bool) -> Self {
        match (palette, dark) {
            (Palette::Coral, false) => tokens(
                0xFAF7_F8, 0xF4EC_EE, 0xE9DE_E1, 0x3031_43, 0x5555_65, 0x7471_7C,
                0x9F59_64, 0x5B75_6B, 0x806C_4F, 0x9E55_59, 0xD9D2_D6, false,
            ),
            (Palette::Coral, true) => tokens(
                0x1716_1C, 0x2C29_31, 0x3A36_3F, 0xF8F2_F4, 0xA197_9D, 0x786F_75,
                0xD18A_94, 0x86A3_99, 0xBBA2_7B, 0xD277_79, 0x413C_41, true,
            ),
            (Palette::Indigo, false) => tokens(
                0xF4F5_F8, 0xECEF_F5, 0xE0E4_ED, 0x191D_26, 0x454C_5B, 0x6870_80,
                0x405D_87, 0x5575_6F, 0xAF8A_55, 0xA74B_52, 0xD2D6_DF, false,
            ),
            (Palette::Indigo, true) => tokens(
                0x1215_1B, 0x252A_34, 0x3137_43, 0xF3F5_F8, 0x959C_A8, 0x6D75_81,
                0x8FA8_CF, 0x84A7_9E, 0xC3A4_77, 0xD184_8A, 0x3A41_4C, true,
            ),
            (Palette::Graphite, false) => tokens(
                0xF6F7_F8, 0xF0F2_F4, 0xE3E6_EA, 0x171A_1F, 0x4147_51, 0x6870_7C,
                0x4F65_78, 0x5B75_6B, 0x806C_4F, 0x9B5E_63, 0xD3D7_DC, false,
            ),
            (Palette::Graphite, true) => tokens(
                0x1113_16, 0x2428_2E, 0x3035_3D, 0xF3F4_F6, 0x979E_A8, 0x6F77_82,
                0x91A5_B7, 0x86A3_99, 0xBDA6_7E, 0xC989_8D, 0x3B41_49, true,
            ),
            (Palette::Mono, false) => tokens(
                0xFAFA_F9, 0xF3F3_F1, 0xE8E8_E5, 0x1111_10, 0x4545_42, 0x6F6F_69,
                0x3436_3A, 0x616D_67, 0x756A_58, 0x815C_60, 0xD8D8_D3, false,
            ),
            (Palette::Mono, true) => tokens(
                0x1111_10, 0x2424_22, 0x3030_2D, 0xFAFA_F9, 0x9A9A_94, 0x7171_6C,
                0xD8DA_DD, 0xA0AA_A5, 0xB0A5_8F, 0xB996_99, 0x3B3B_38, true,
            ),
            (Palette::Circuit, false) => tokens(
                0xF4F5_F5, 0xECEE_EE, 0xDDE1_E1, 0x161A_1A, 0x4148_48, 0x6871_71,
                0x3E6C_70, 0x4F73_68, 0x7D6C_52, 0x995D_61, 0xCDD2_D2, false,
            ),
            (Palette::Circuit, true) => tokens(
                0x1014_14, 0x232A_2A, 0x3038_38, 0xF1F4_F4, 0x929D_9D, 0x6975_75,
                0x7FAD_AF, 0x7DA3_94, 0xBAA2_7B, 0xC982_86, 0x3942_42, true,
            ),
            (Palette::Dusk, false) => tokens(
                0xF7F6_F8, 0xF0EE_F2, 0xE4E1_E7, 0x1D1B_20, 0x4B47_50, 0x716C_76,
                0x675F_78, 0x5D74_6C, 0x806C_4F, 0x995F_68, 0xD6D1_D9, false,
            ),
            (Palette::Dusk, true) => tokens(
                0x1513_17, 0x2A26_2E, 0x3833_3D, 0xF5F2_F6, 0x9E95_A2, 0x756C_79,
                0xA9A1_BA, 0x82A0_97, 0xB6A0_78, 0xC681_8A, 0x403A_44, true,
            ),
        }
    }

    pub const fn dark() -> Self {
        Self::of(Palette::Coral, true)
    }

    pub const fn light() -> Self {
        Self::of(Palette::Coral, false)
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
            dark: false,
        }
    }

    #[must_use]
    pub fn is_dark(self) -> bool {
        self.dark
    }

    #[must_use]
    pub fn current() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return Self::plain();
        }
        let palette = palette();
        match scheme() {
            Scheme::Plain => Self::plain(),
            Scheme::Dark => Self::of(palette, true),
            Scheme::Light => Self::of(palette, false),
            Scheme::Auto => Self::of(palette, !terminal_is_light()),
        }
    }

    /// OSC 12 cursor matching the active primary.
    #[must_use]
    pub fn cursor_osc(self) -> String {
        match self.rose {
            Color::Rgb(r, g, b) => format!("\x1b]12;#{r:02X}{g:02X}{b:02X}\x07"),
            _ => CURSOR_RESET.to_owned(),
        }
    }

    /// OSC 11 default terminal background matching `surface.canvas`.
    #[must_use]
    pub fn background_osc(self) -> String {
        match self.bg {
            Color::Rgb(r, g, b) => format!("\x1b]11;#{r:02X}{g:02X}{b:02X}\x07"),
            _ => BACKGROUND_RESET.to_owned(),
        }
    }

    /// Cursor + default-background OSC for the current palette.
    #[must_use]
    pub fn terminal_osc(self) -> String {
        format!("{}{}", self.background_osc(), self.cursor_osc())
    }

    #[must_use]
    pub fn fg(self, color: Color) -> Style {
        Style::default().fg(color)
    }

    #[must_use]
    pub fn base(self) -> Style {
        Style::default().fg(self.text).bg(self.bg)
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

pub const CURSOR_RESET: &str = "\x1b]112\x07";
pub const BACKGROUND_RESET: &str = "\x1b]111\x07";
pub const TERMINAL_RESET: &str = "\x1b]111\x07\x1b]112\x07";

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
        assert_eq!(theme.rose, hex(0xD18A_94));
        assert_eq!(theme.sage, hex(0x86A3_99));
        assert!(theme.is_dark());
    }

    #[test]
    fn light_matches_coral_tokens() {
        let theme = Theme::light();
        assert_eq!(theme.bg, hex(0xFAF7_F8));
        assert_eq!(theme.rose, hex(0x9F59_64));
        assert_eq!(theme.sage, hex(0x5B75_6B));
        assert!(!theme.is_dark());
        assert_eq!(theme.base().bg, Some(hex(0xFAF7_F8)));
        assert!(theme.background_osc().contains("#FAF7F8"));
    }

    #[test]
    fn indigo_and_dusk_are_distinct_from_coral() {
        let coral = Theme::of(Palette::Coral, false);
        let indigo = Theme::of(Palette::Indigo, false);
        let dusk = Theme::of(Palette::Dusk, false);
        assert_ne!(coral.rose, indigo.rose);
        assert_ne!(coral.rose, dusk.rose);
        assert_eq!(indigo.rose, hex(0x405D_87));
        assert_eq!(dusk.rose, hex(0x675F_78));
        assert_eq!(Theme::of(Palette::Circuit, true).rose, hex(0x7FAD_AF));
        assert_eq!(Theme::of(Palette::Mono, false).rose, hex(0x3436_3A));
        assert_eq!(Theme::of(Palette::Graphite, true).rose, hex(0x91A5_B7));
    }

    #[test]
    fn every_preset_has_six_zh_names() {
        assert_eq!(Palette::ALL.len(), 6);
        assert_eq!(Palette::parse("暮色"), Some(Palette::Dusk));
        assert_eq!(Palette::parse("indigo"), Some(Palette::Indigo));
    }

    #[test]
    fn pref_parses_palette_scheme_and_pair() {
        assert_eq!(Scheme::parse("dark"), Some(Scheme::Dark));
        assert_eq!(Palette::parse("dusk"), Some(Palette::Dusk));
        assert_eq!(Scheme::parse("dusk"), None);
        let pair = ThemePref::parse("indigo-dark").unwrap();
        assert_eq!(pair.palette, Palette::Indigo);
        assert_eq!(pair.scheme, Scheme::Dark);
    }

    #[test]
    fn plain_uses_terminal_colors() {
        let theme = Theme::plain();
        assert_eq!(theme.bg, Color::Reset);
        assert_eq!(theme.rose, Color::Reset);
    }

    #[test]
    fn colorfgbg_15_is_light() {
        assert!(matches!("15", "7" | "15"));
        assert!(!matches!("0", "7" | "15"));
    }
}
