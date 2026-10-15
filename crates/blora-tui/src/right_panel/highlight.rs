// SPDX-License-Identifier: GPL-3.0-or-later
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::{path::Path, sync::OnceLock};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines)
}
fn syntax(path: &Path, text: &str) -> &'static SyntaxReference {
    let set = syntaxes();
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| set.find_syntax_by_extension(name))
        .or_else(|| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .and_then(|ext| set.find_syntax_by_extension(ext))
        })
        .or_else(|| {
            text.lines()
                .next()
                .and_then(|line| set.find_syntax_by_first_line(line))
        })
        .unwrap_or_else(|| set.find_syntax_plain_text())
}
pub(super) fn language(path: &Path, text: &str) -> &'static str {
    &syntax(path, text).name
}
pub(super) fn lines(path: &Path, text: &str, dark: bool) -> Vec<Line<'static>> {
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    let themes = THEMES.get_or_init(ThemeSet::load_defaults);
    let theme = &themes.themes[if dark {
        "base16-ocean.dark"
    } else {
        "InspiredGitHub"
    }];
    let mut highlighter = HighlightLines::new(syntax(path, text), theme);
    let mut output = Vec::new();
    for line in LinesWithEndings::from(text) {
        let spans = match highlighter.highlight_line(line, syntaxes()) {
            Ok(ranges) => ranges
                .into_iter()
                .map(|(highlight, text)| {
                    let mut style = Style::default().fg(Color::Rgb(
                        highlight.foreground.r,
                        highlight.foreground.g,
                        highlight.foreground.b,
                    ));
                    if highlight.font_style.contains(FontStyle::BOLD) {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if highlight.font_style.contains(FontStyle::ITALIC) {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if highlight.font_style.contains(FontStyle::UNDERLINE) {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    Span::styled(
                        text.trim_end_matches(['\r', '\n']).replace('\t', "    "),
                        style,
                    )
                })
                .collect(),
            Err(_) => vec![Span::raw(
                line.trim_end_matches(['\r', '\n']).replace('\t', "    "),
            )],
        };
        output.push(Line::from(spans));
    }
    if text.is_empty() || text.ends_with('\n') {
        output.push(Line::default());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_extension_filename_shebang_and_plain_text() {
        assert_eq!(language(Path::new("test.rs"), ""), "Rust");
        assert_ne!(language(Path::new("Makefile"), "all:"), "Plain Text");
        assert_eq!(
            language(Path::new("script"), "#!/usr/bin/env python\nprint(1)"),
            "Python"
        );
        assert_eq!(language(Path::new("unknown.blora"), "hello"), "Plain Text");
    }
    #[test]
    fn both_themes_keep_content_and_final_line() {
        for dark in [true, false] {
            let lines = lines(Path::new("test.rs"), "let x = 1;\n", dark);
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0].to_string(), "let x = 1;");
        }
    }
}
