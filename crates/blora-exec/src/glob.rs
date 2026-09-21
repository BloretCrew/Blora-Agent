// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Minimal glob matching for workspace file lookups.
//!
//! Supported: `*` (within a segment), `?`, and `**` (zero or more segments).
//! Patterns without a `/` match the file name anywhere in the tree, like
//! `find -name`; patterns with a `/` match the path relative to the search root.

/// True when `path` (forward-slash separated, relative) matches `pattern`.
#[must_use]
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim().trim_start_matches("./");
    let path = path.trim_start_matches("./");
    if pattern.is_empty() {
        return false;
    }
    if !pattern.contains('/') {
        let name = path.rsplit('/').next().unwrap_or(path);
        return wildcard(pattern, name);
    }
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match_segments(&pat, &segs)
}

fn match_segments(pat: &[&str], path: &[&str]) -> bool {
    match pat.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| match_segments(rest, &path[skip..])),
        Some((first, rest)) => match path.split_first() {
            Some((seg, path_rest)) => wildcard(first, seg) && match_segments(rest, path_rest),
            None => false,
        },
    }
}

/// `*` and `?` wildcard match on a single path segment.
fn wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // Classic iterative matcher with backtracking to the last `*`.
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_patterns_match_file_names_anywhere() {
        assert!(glob_match("*.rs", "src/lib.rs"));
        assert!(glob_match("*.rs", "lib.rs"));
        assert!(!glob_match("*.rs", "src/lib.ts"));
        assert!(glob_match("Cargo.*", "crates/a/Cargo.toml"));
        assert!(glob_match("l?b.rs", "x/lib.rs"));
    }

    #[test]
    fn slashed_patterns_match_relative_paths() {
        assert!(glob_match("src/*.rs", "src/lib.rs"));
        assert!(!glob_match("src/*.rs", "src/a/lib.rs"));
        assert!(glob_match("src/**/*.rs", "src/a/b/lib.rs"));
        assert!(glob_match("src/**/*.rs", "src/lib.rs"));
        assert!(glob_match("**/*.md", "README.md"));
        assert!(glob_match("**/docs/*.md", "a/docs/x.md"));
        assert!(!glob_match("**/docs/*.md", "a/doc/x.md"));
        assert!(glob_match("crates/*/src/**", "crates/x/src/a/b.rs"));
        assert!(!glob_match("", "a"));
    }

    #[test]
    fn wildcard_backtracks() {
        assert!(wildcard("a*b*c", "axxbyyc"));
        assert!(!wildcard("a*b*c", "axxbyy"));
        assert!(wildcard("*", ""));
        assert!(wildcard("**", "anything"));
        assert!(!wildcard("a?", "a"));
    }
}
