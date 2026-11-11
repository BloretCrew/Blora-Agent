// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Read-only, viewport-highlighted text preview.

use crate::theme::Theme;
use ratatui::{style::Style, text::Line};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use super::{
    highlight::{CachedLine, HighlightCache, Segment},
    render,
};

pub(crate) struct Preview {
    path: PathBuf,
    text: String,
    line_starts: Vec<usize>,
    cache: RefCell<HighlightCache>,
    max_width: std::cell::Cell<Option<usize>>,
}

impl Preview {
    pub(crate) fn new(path: &Path, text: String, dark: bool) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(memchr::memchr_iter(b'\n', text.as_bytes()).map(|index| index + 1));
        let cache = HighlightCache::new(path, &text, dark);
        Self {
            path: path.to_path_buf(),
            text,
            line_starts,
            cache: RefCell::new(cache),
            max_width: std::cell::Cell::new(None),
        }
    }

    pub(crate) fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub(crate) fn line_text(&self, index: usize) -> &str {
        let start = self.line_starts[index];
        let end = self
            .line_starts
            .get(index + 1)
            .copied()
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\r', '\n'])
    }

    pub(crate) fn language(&self) -> &'static str {
        self.cache.borrow().language()
    }

    pub(crate) fn ensure_viewport(&self, first: usize, rows: usize, dark: bool) {
        let mut cache = self.cache.borrow_mut();
        if cache.is_dark() != dark {
            *cache = HighlightCache::new(&self.path, &self.text, dark);
            self.max_width.set(None);
        }
        cache.ensure(&self.text, &self.line_starts, first, rows, false);
    }

    pub(crate) fn max_width(&self, dark: bool) -> usize {
        if let Some(width) = self.max_width.get() {
            return width;
        }
        let mut cache = self.cache.borrow_mut();
        if cache.is_dark() != dark {
            *cache = HighlightCache::new(&self.path, &self.text, dark);
        }
        let width = cache.max_width(&self.text, &self.line_starts);
        self.max_width.set(Some(width));
        width
    }

    pub(crate) fn render_line(
        &self,
        index: usize,
        width: usize,
        horizontal: usize,
        theme: &Theme,
    ) -> Line<'static> {
        let line = self
            .cache
            .borrow()
            .line(index)
            .cloned()
            .unwrap_or_else(|| CachedLine {
                segments: vec![Segment {
                    style: Style::default(),
                    text: self.line_text(index).to_owned(),
                }],
                plain: true,
            });
        let plain = theme.fg(theme.text);
        render::render(&line, plain, horizontal, width, Style::default(), &[])
    }
}
