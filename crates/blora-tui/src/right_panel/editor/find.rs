// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Incremental in-file find/replace state.

use regex::RegexBuilder;
use std::ops::Range;

const MAX_MATCHES: usize = 200_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FindFocus {
    Query,
    Replacement,
}

#[derive(Clone, Default)]
pub(super) struct FindState {
    pub(super) open: bool,
    pub(super) replace: bool,
    pub(super) query: String,
    pub(super) replacement: String,
    pub(super) focus: Option<FindFocus>,
    pub(super) matches: Vec<(usize, usize)>,
    pub(super) current: Option<usize>,
    pub(super) case_sensitive: Option<bool>,
    pub(super) error: Option<String>,
}

impl FindState {
    pub(super) fn open_find(&mut self, initial: &str) {
        if !self.open && !initial.is_empty() {
            self.query.clear();
            self.query.push_str(initial.lines().next().unwrap_or(""));
        }
        self.open = true;
        self.replace = false;
        self.focus = Some(FindFocus::Query);
        self.current = None;
    }

    pub(super) fn open_replace(&mut self, initial: &str) {
        if !self.open && !initial.is_empty() {
            self.query.clear();
            self.query.push_str(initial.lines().next().unwrap_or(""));
        }
        self.open = true;
        self.replace = true;
        self.focus = Some(FindFocus::Query);
        self.current = None;
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.replace = false;
        self.focus = None;
        self.current = None;
        self.error = None;
    }

    pub(super) fn focus_mut(&mut self) -> Option<&mut String> {
        match self.focus? {
            FindFocus::Query => Some(&mut self.query),
            FindFocus::Replacement => Some(&mut self.replacement),
        }
    }

    pub(super) fn toggle_focus(&mut self) {
        if !self.replace {
            return;
        }
        self.focus = match self.focus {
            Some(FindFocus::Query) => Some(FindFocus::Replacement),
            Some(FindFocus::Replacement) => Some(FindFocus::Query),
            None => Some(FindFocus::Query),
        };
    }

    pub(super) fn toggle_case(&mut self, text: &str) {
        let current = self.case_sensitive.unwrap_or_else(|| self.smart_case());
        self.case_sensitive = Some(!current);
        self.refresh(text, 0);
    }

    fn smart_case(&self) -> bool {
        self.query.chars().any(char::is_uppercase)
    }

    pub(super) fn case_sensitive(&self) -> bool {
        self.case_sensitive.unwrap_or_else(|| self.smart_case())
    }

    pub(super) fn refresh(&mut self, text: &str, cursor: usize) {
        self.matches.clear();
        self.current = None;
        self.error = None;
        if self.query.is_empty() {
            return;
        }
        let pattern = regex::escape(&self.query);
        let built = RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive())
            .build();
        let regex = match built {
            Ok(regex) => regex,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        for found in regex.find_iter(text).take(MAX_MATCHES) {
            if found.start() == found.end() {
                continue;
            }
            self.matches.push((found.start(), found.end()));
        }
        if self.matches.is_empty() {
            return;
        }
        let index = self
            .matches
            .iter()
            .position(|(start, _)| *start >= cursor)
            .unwrap_or(0);
        self.current = Some(index);
    }

    pub(super) fn current(&self) -> Option<Range<usize>> {
        let index = self.current?;
        let (start, end) = *self.matches.get(index)?;
        Some(start..end)
    }

    pub(super) fn step(&mut self, forward: bool) {
        if self.matches.is_empty() {
            return;
        }
        let current = self.current.unwrap_or(0);
        let next = if forward {
            (current + 1) % self.matches.len()
        } else if current == 0 {
            self.matches.len() - 1
        } else {
            current - 1
        };
        self.current = Some(next);
    }

    /// Refresh and pick the match nearest to `cursor` (used after edits).
    pub(super) fn refresh_near(&mut self, text: &str, cursor: usize) {
        let previous = self.current;
        self.refresh(text, cursor);
        if self.current.is_none() && !self.matches.is_empty() {
            self.current = previous.map(|index| index.min(self.matches.len() - 1));
        }
    }

    pub(super) fn all_ranges(&self) -> Vec<(usize, usize)> {
        self.matches.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_literal_text_and_smart_case() {
        let text = "alpha Alpha ALPHA";
        let mut find = FindState::default();
        find.open_find("");
        find.query = "alpha".into();
        find.refresh(text, 0);
        assert_eq!(find.matches.len(), 3);
        find.query = "Alpha".into();
        find.case_sensitive = None;
        find.refresh(text, 0);
        assert_eq!(find.matches.len(), 1);
    }

    #[test]
    fn escapes_regex_characters() {
        let text = "a.b a*b ab";
        let mut find = FindState::default();
        find.open_find("");
        find.query = "a.".into();
        find.refresh(text, 0);
        assert_eq!(find.matches, vec![(0, 2)]);
    }

    #[test]
    fn steps_wrap_both_directions() {
        let text = "x x x";
        let mut find = FindState::default();
        find.open_find("");
        find.query = "x".into();
        find.refresh(text, 0);
        assert_eq!(find.current, Some(0));
        find.step(true);
        assert_eq!(find.current, Some(1));
        find.step(false);
        assert_eq!(find.current, Some(0));
        find.step(false);
        assert_eq!(find.current, Some(2));
    }
}
