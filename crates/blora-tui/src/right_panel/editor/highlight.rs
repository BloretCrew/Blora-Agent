// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Viewport-only, checkpointed syntect highlighting.

use ratatui::style::{Color, Modifier, Style};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, HighlightIterator, HighlightState, Highlighter, Theme, ThemeSet},
    parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet},
};

pub(super) const CHECKPOINT_LINES: usize = 64;
pub(super) const MAX_HIGHLIGHT_LINE_BYTES: usize = 16 * 1024;
pub(super) const MAX_PARSE_LINES_PER_FRAME: usize = 512;
pub(super) const OVERSCAN: usize = 8;

#[derive(Clone)]
pub(super) struct Segment {
    pub(super) style: Style,
    pub(super) text: String,
}

#[derive(Clone)]
pub(super) struct CachedLine {
    pub(super) segments: Vec<Segment>,
    pub(super) plain: bool,
}

pub(super) struct WarmCheckpoint {
    pub(super) line: usize,
    pub(super) state: HighlightState,
    pub(super) parse: ParseState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct EnsureResult {
    pub(super) processed: usize,
    pub(super) ready_end: usize,
}

pub(super) struct HighlightCache {
    path: PathBuf,
    dark: bool,
    syntax: &'static SyntaxReference,
    lines: Vec<Option<CachedLine>>,
    checkpoints: BTreeMap<usize, (HighlightState, ParseState)>,
    valid_until: usize,
    max_width: Option<usize>,
}

impl HighlightCache {
    pub(super) fn new(path: &Path, text: &str, dark: bool) -> Self {
        let syntax = syntax(path, text);
        let mut cache = Self {
            path: path.to_path_buf(),
            dark,
            syntax,
            lines: Vec::new(),
            checkpoints: BTreeMap::new(),
            valid_until: 0,
            max_width: None,
        };
        cache.reset(path, syntax, dark);
        cache
    }

    fn reset(&mut self, path: &Path, syntax: &'static SyntaxReference, dark: bool) {
        self.path = path.to_path_buf();
        self.syntax = syntax;
        self.dark = dark;
        self.lines.clear();
        self.checkpoints.clear();
        let theme = theme(dark);
        let (highlight, parse) = HighlightLines::new(syntax, theme).state();
        self.checkpoints.insert(0, (highlight, parse));
        self.valid_until = 0;
        self.max_width = None;
    }

    pub(super) fn language(&self) -> &'static str {
        self.syntax.name.as_str()
    }

    pub(super) fn is_dark(&self) -> bool {
        self.dark
    }

    pub(super) fn line(&self, index: usize) -> Option<&CachedLine> {
        self.lines.get(index).and_then(Option::as_ref)
    }

    pub(super) fn invalidate_from(&mut self, line: usize) {
        self.lines.truncate(line);
        self.checkpoints.retain(|key, _| *key <= line);
        self.valid_until = self.valid_until.min(line);
        self.max_width = None;
    }

    pub(super) fn add_checkpoints(&mut self, checkpoints: Vec<WarmCheckpoint>) {
        for checkpoint in checkpoints {
            self.checkpoints
                .entry(checkpoint.line)
                .or_insert((checkpoint.state, checkpoint.parse));
        }
    }

    pub(super) fn ensure(
        &mut self,
        text: &str,
        line_starts: &[usize],
        first: usize,
        count: usize,
        warming: bool,
    ) -> EnsureResult {
        let total = line_starts.len();
        if total == 0 || count == 0 {
            return EnsureResult::default();
        }
        let first = first.min(total.saturating_sub(1));
        let visible_end = (first + count).min(total);
        let target_end = (visible_end + OVERSCAN).min(total);
        if self.lines.len() < total {
            self.lines.resize(total, None);
        }
        if self.valid_until >= visible_end
            && self.lines[first..visible_end].iter().all(Option::is_some)
        {
            return EnsureResult {
                processed: 0,
                ready_end: visible_end,
            };
        }
        let start = if self.valid_until > first {
            self.valid_until
        } else {
            first
        };
        let checkpoint_line = self
            .checkpoints
            .range(..=start)
            .next_back()
            .map(|(line, _)| *line)
            .unwrap_or(0);
        if warming && checkpoint_line.saturating_add(MAX_PARSE_LINES_PER_FRAME) < target_end {
            return EnsureResult {
                processed: 0,
                ready_end: first,
            };
        }
        let (state, parse) = self
            .checkpoints
            .get(&checkpoint_line)
            .cloned()
            .unwrap_or_else(|| HighlightLines::new(self.syntax, theme(self.dark)).state());
        let mut highlight = HighlightLines::from_state(theme(self.dark), state, parse);
        let budget_end = checkpoint_line
            .saturating_add(MAX_PARSE_LINES_PER_FRAME)
            .min(total);
        let end = target_end.max(start).min(budget_end);
        let mut processed = 0;
        for line in checkpoint_line..end {
            if line % CHECKPOINT_LINES == 0 {
                let (state, parse) = highlight.state();
                self.checkpoints
                    .insert(line, (state.clone(), parse.clone()));
                highlight = HighlightLines::from_state(theme(self.dark), state, parse);
            }
            let cached = self.highlight_line(line, text, line_starts, &mut highlight);
            if self.lines[line].is_none() {
                self.lines[line] = Some(cached);
                processed += 1;
            }
        }
        let (state, parse) = highlight.state();
        self.checkpoints.insert(end, (state, parse));
        if end > self.valid_until {
            self.valid_until = end;
        }
        EnsureResult {
            processed,
            ready_end: end.min(visible_end),
        }
    }

    fn highlight_line(
        &self,
        line: usize,
        text: &str,
        line_starts: &[usize],
        highlight: &mut HighlightLines<'static>,
    ) -> CachedLine {
        let start = line_starts[line];
        let end = line_starts.get(line + 1).copied().unwrap_or(text.len());
        let raw = &text[start..end];
        if raw.len() > MAX_HIGHLIGHT_LINE_BYTES {
            return CachedLine {
                segments: vec![Segment {
                    style: Style::default(),
                    text: raw.trim_end_matches(['\r', '\n']).to_owned(),
                }],
                plain: true,
            };
        }
        let ranges = match highlight.highlight_line(raw, syntaxes()) {
            Ok(ranges) => ranges,
            Err(_) => {
                return CachedLine {
                    segments: vec![Segment {
                        style: Style::default(),
                        text: raw.trim_end_matches(['\r', '\n']).to_owned(),
                    }],
                    plain: true,
                };
            }
        };
        let mut segments = Vec::with_capacity(ranges.len());
        for (syntax_style, piece) in ranges {
            let piece = piece.trim_end_matches(['\r', '\n']);
            if piece.is_empty() {
                continue;
            }
            let mut style = Style::default().fg(Color::Rgb(
                syntax_style.foreground.r,
                syntax_style.foreground.g,
                syntax_style.foreground.b,
            ));
            if syntax_style.font_style.contains(FontStyle::BOLD) {
                style = style.add_modifier(Modifier::BOLD);
            }
            if syntax_style.font_style.contains(FontStyle::ITALIC) {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if syntax_style.font_style.contains(FontStyle::UNDERLINE) {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            segments.push(Segment {
                style,
                text: piece.to_owned(),
            });
        }
        CachedLine {
            segments,
            plain: false,
        }
    }

    /// Highlight every line once to build parse checkpoints only.
    #[cfg(test)]
    pub(super) fn warm(&mut self, text: &str, line_starts: &[usize], cancelled: impl Fn() -> bool) {
        let mut checkpoints = Vec::new();
        compute_checkpoints(
            text,
            line_starts,
            self.syntax,
            self.dark,
            &mut |chunk, _done| {
                checkpoints.extend(chunk);
                !cancelled()
            },
        );
        self.add_checkpoints(checkpoints);
    }

    pub(super) fn max_width(&mut self, text: &str, line_starts: &[usize]) -> usize {
        if let Some(width) = self.max_width {
            return width;
        }
        let mut width = 0;
        for line in 0..line_starts.len() {
            let start = line_starts[line];
            let end = line_starts.get(line + 1).copied().unwrap_or(text.len());
            width = width.max(super::document::display_width(
                text[start..end].trim_end_matches(['\r', '\n']),
            ));
        }
        self.max_width = Some(width);
        width
    }
}

pub(super) fn compute_checkpoints(
    text: &str,
    line_starts: &[usize],
    syntax: &'static SyntaxReference,
    dark: bool,
    emit: &mut impl FnMut(Vec<WarmCheckpoint>, bool) -> bool,
) {
    let highlighter = Highlighter::new(theme(dark));
    let mut highlight_state = HighlightState::new(&highlighter, ScopeStack::new());
    let mut parse_state = ParseState::new(syntax);
    let mut batch = Vec::new();
    for line in 0..line_starts.len() {
        if line % CHECKPOINT_LINES == 0 {
            batch.push(WarmCheckpoint {
                line,
                state: highlight_state.clone(),
                parse: parse_state.clone(),
            });
            if batch.len() >= 16 && !emit(std::mem::take(&mut batch), false) {
                return;
            }
        }
        let start = line_starts[line];
        let end = line_starts.get(line + 1).copied().unwrap_or(text.len());
        let raw = &text[start..end];
        if let Ok(ops) = parse_state.parse_line(raw, syntaxes()) {
            for _ in HighlightIterator::new(&mut highlight_state, &ops, raw, &highlighter) {}
        }
    }
    batch.push(WarmCheckpoint {
        line: line_starts.len(),
        state: highlight_state,
        parse: parse_state,
    });
    let _ = emit(batch, true);
}

pub(super) fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines)
}

pub(super) fn themes() -> &'static ThemeSet {
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    THEMES.get_or_init(ThemeSet::load_defaults)
}

pub(super) fn theme(dark: bool) -> &'static Theme {
    &themes().themes[if dark {
        "base16-ocean.dark"
    } else {
        "InspiredGitHub"
    }]
}

pub(super) fn syntax(path: &Path, text: &str) -> &'static SyntaxReference {
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
    syntax(path, text).name.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(text: &str) -> Vec<usize> {
        let mut starts = vec![0];
        starts.extend(memchr::memchr_iter(b'\n', text.as_bytes()).map(|index| index + 1));
        starts
    }

    #[test]
    fn detects_language_and_themes_keep_lines() {
        assert_eq!(language(Path::new("test.rs"), ""), "Rust");
        let text = "let x = 1;\n";
        let starts = starts(text);
        let mut cache = HighlightCache::new(Path::new("test.rs"), text, true);
        let result = cache.ensure(text, &starts, 0, 4, false);
        assert!(result.processed > 0);
        assert!(cache.line(0).is_some());
        assert_eq!(cache.language(), "Rust");
    }

    #[test]
    fn second_ensure_does_no_work() {
        let text: String = (0..200).map(|line| format!("let x = {line};\n")).collect();
        let starts = starts(&text);
        let mut cache = HighlightCache::new(Path::new("test.rs"), &text, true);
        assert!(cache.ensure(&text, &starts, 0, 20, false).processed > 0);
        assert_eq!(cache.ensure(&text, &starts, 0, 20, false).processed, 0);
    }

    #[test]
    fn viewport_far_down_uses_checkpoint() {
        let text: String = (0..20_000)
            .map(|line| format!("let x = {line};\n"))
            .collect();
        let starts = starts(&text);
        let mut cache = HighlightCache::new(Path::new("test.rs"), &text, true);
        cache.warm(&text, &starts, || false);
        let result = cache.ensure(&text, &starts, 10_000, 30, false);
        assert!(result.processed <= 30 + CHECKPOINT_LINES + OVERSCAN);
    }

    #[test]
    fn edits_invalidate_suffix_only() {
        let text: String = (0..1000).map(|line| format!("let x = {line};\n")).collect();
        let starts = starts(&text);
        let mut cache = HighlightCache::new(Path::new("test.rs"), &text, true);
        cache.ensure(&text, &starts, 0, 50, false);
        cache.invalidate_from(10);
        assert!(cache.lines.len() <= 10);
        assert!(cache.checkpoints.keys().all(|key| *key <= 10));
    }
}
