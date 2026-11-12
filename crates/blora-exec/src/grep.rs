// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Rich content search used by the `grep` tool.

use std::fs;
use std::path::Path;

use blora_types::{BloraError, Result};
use globset::{Glob, GlobMatcher};
use ignore::WalkBuilder;
use regex::Regex;

use crate::LocalBackend;

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_RESULTS: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    Content,
    FilesWithMatches,
    Count,
}

#[derive(Clone, Debug)]
pub struct GrepOptions {
    pub pattern: String,
    pub path: String,
    pub include: Option<String>,
    pub exclude: Option<String>,
    pub case_sensitive: Option<bool>,
    pub smart_case: bool,
    pub fixed_strings: bool,
    pub multiline: bool,
    pub context: Option<usize>,
    pub before: Option<usize>,
    pub after: Option<usize>,
    pub output_mode: OutputMode,
    pub file_type: Option<String>,
    pub max_results: usize,
    pub hidden: bool,
    pub no_ignore: bool,
}

impl LocalBackend {
    pub fn grep(&self, options: &GrepOptions) -> Result<String> {
        self.policy().require(self.policy().search(), "grep")?;
        if options.pattern.is_empty() {
            return Err(BloraError::Other("grep needs a pattern".to_owned()));
        }
        let root = self.policy().resolve(&options.path)?;
        let include = compile_glob(options.include.as_deref())?;
        let exclude = compile_glob(options.exclude.as_deref())?;
        let types = options.file_type.as_deref().map(type_extensions);
        let regex = compile_regex(options)?;
        let before = options.before.or(options.context).unwrap_or(0);
        let after = options.after.or(options.context).unwrap_or(0);
        let max_results = options.max_results.clamp(1, MAX_RESULTS);

        let mut files = Vec::new();
        let walker = WalkBuilder::new(&root)
            .hidden(!options.hidden)
            .git_ignore(!options.no_ignore)
            .git_global(!options.no_ignore)
            .git_exclude(!options.no_ignore)
            .build();
        for entry in walker.flatten() {
            let path = entry.path();
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if !options.no_ignore && skip_dir(path) {
                continue;
            }
            let relative = path
                .strip_prefix(self.policy().workspace())
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            if !options.no_ignore && skip_relative(&relative) {
                continue;
            }
            if let Some(include) = &include {
                if !include.is_match(&relative) {
                    continue;
                }
            }
            if let Some(exclude) = &exclude {
                if exclude.is_match(&relative) {
                    continue;
                }
            }
            if let Some(types) = &types {
                let matches = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| types.contains(&ext.to_ascii_lowercase()));
                if !matches {
                    continue;
                }
            }
            files.push((path.to_path_buf(), relative));
        }
        files.sort_by(|a, b| a.1.cmp(&b.1));

        let mut lines = Vec::new();
        for (path, relative) in files {
            if lines.len() >= max_results {
                break;
            }
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            if metadata.len() > MAX_FILE_BYTES {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            if bytes.contains(&0) {
                continue;
            }
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            let matches = match options.output_mode {
                OutputMode::Content => content_matches(
                    &text,
                    &regex,
                    &relative,
                    before,
                    after,
                    options.multiline,
                    max_results - lines.len(),
                ),
                OutputMode::FilesWithMatches => {
                    if regex.is_match(&text) {
                        vec![relative.clone()]
                    } else {
                        Vec::new()
                    }
                }
                OutputMode::Count => {
                    let count = regex.find_iter(&text).count();
                    if count > 0 {
                        vec![format!("{relative}:{count}")]
                    } else {
                        Vec::new()
                    }
                }
            };
            lines.extend(matches);
        }
        if lines.is_empty() {
            return Ok("no matches".to_owned());
        }
        let truncated = lines.len() >= max_results;
        let mut out = lines.join("\n");
        if truncated {
            out.push_str(&format!(
                "\n… truncated at {max_results} results; narrow pattern/include or raise max_results …"
            ));
        }
        Ok(out)
    }
}

fn compile_glob(pattern: Option<&str>) -> Result<Option<GlobMatcher>> {
    pattern
        .map(|pattern| {
            Glob::new(pattern)
                .map(|glob| glob.compile_matcher())
                .map_err(|err| BloraError::Other(format!("invalid glob {pattern}: {err}")))
        })
        .transpose()
}

fn compile_regex(options: &GrepOptions) -> Result<Regex> {
    let mut pattern = if options.fixed_strings {
        regex::escape(&options.pattern)
    } else {
        options.pattern.clone()
    };
    if options.multiline {
        pattern = format!("(?s:{pattern})");
    }
    let case_sensitive = options
        .case_sensitive
        .unwrap_or_else(|| options.smart_case && options.pattern.chars().any(char::is_uppercase));
    if !case_sensitive {
        pattern = format!("(?i:{pattern})");
    }
    Regex::new(&pattern).map_err(|err| BloraError::Other(format!("invalid pattern: {err}")))
}

fn content_matches(
    text: &str,
    regex: &Regex,
    path: &str,
    before: usize,
    after: usize,
    multiline: bool,
    limit: usize,
) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut hit_lines = Vec::new();
    if multiline {
        for hit in regex.find_iter(text) {
            let line = text[..hit.start()]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count();
            hit_lines.push(line);
        }
    } else {
        for (index, line) in lines.iter().enumerate() {
            if regex.is_match(line) {
                hit_lines.push(index);
            }
        }
    }
    hit_lines.sort_unstable();
    hit_lines.dedup();
    let mut out = Vec::new();
    let mut last_printed: Option<usize> = None;
    for line in hit_lines {
        let start = line.saturating_sub(before);
        let end = (line + after + 1).min(lines.len());
        if let Some(last) = last_printed {
            if start > last + 1 && !out.is_empty() {
                out.push("--".to_owned());
            }
        }
        for (offset, text) in lines[start..end].iter().enumerate() {
            let index = start + offset;
            if last_printed.is_some_and(|last| index <= last) {
                continue;
            }
            let separator = if index == line { ":" } else { "-" };
            out.push(format!("{path}{separator}{}:{}", index + 1, text));
            last_printed = Some(index);
            if out.len() >= limit {
                return out;
            }
        }
        if out.len() >= limit {
            return out;
        }
    }
    out
}

fn type_extensions(kind: &str) -> Vec<String> {
    let extensions: &[&str] = match kind {
        "rust" | "rs" => &["rs"],
        "typescript" | "ts" => &["ts", "tsx", "mts", "cts"],
        "javascript" | "js" => &["js", "jsx", "mjs", "cjs"],
        "python" | "py" => &["py", "pyi"],
        "go" => &["go"],
        "java" => &["java"],
        "json" => &["json"],
        "markdown" | "md" => &["md", "mdx"],
        "yaml" | "yml" => &["yaml", "yml"],
        "toml" => &["toml"],
        "shell" | "sh" => &["sh", "bash", "zsh"],
        "html" => &["html", "htm"],
        "css" => &["css", "scss", "sass", "less"],
        other => return vec![other.trim_start_matches('.').to_ascii_lowercase()],
    };
    extensions.iter().map(|ext| (*ext).to_owned()).collect()
}

fn skip_dir(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(".git" | "target" | "node_modules" | ".blora")
        )
    })
}

fn skip_relative(relative: &str) -> bool {
    relative
        .split('/')
        .any(|component| matches!(component, ".git" | "target" | "node_modules" | ".blora"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LocalBackend;
    use blora_policy::Policy;

    fn backend(dir: &Path) -> LocalBackend {
        LocalBackend::new(Policy::new(dir, true).unwrap())
    }

    fn options(pattern: &str) -> GrepOptions {
        GrepOptions {
            pattern: pattern.to_owned(),
            path: ".".to_owned(),
            include: None,
            exclude: None,
            case_sensitive: None,
            smart_case: true,
            fixed_strings: false,
            multiline: false,
            context: None,
            before: None,
            after: None,
            output_mode: OutputMode::Content,
            file_type: None,
            max_results: 100,
            hidden: true,
            no_ignore: true,
        }
    }

    #[test]
    fn finds_content_with_context_and_modes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "one\nneedle here\nthree\nfour\n").unwrap();
        std::fs::write(dir.path().join("b.py"), "nothing\n").unwrap();
        let backend = backend(dir.path());
        let mut opts = options("needle");
        opts.context = Some(1);
        let text = backend.grep(&opts).unwrap();
        assert!(text.contains("a.rs-1:one"));
        assert!(text.contains("a.rs:2:needle here"));
        assert!(text.contains("a.rs-3:three"));

        opts.output_mode = OutputMode::FilesWithMatches;
        assert_eq!(backend.grep(&opts).unwrap(), "a.rs");

        opts.output_mode = OutputMode::Count;
        assert_eq!(backend.grep(&opts).unwrap(), "a.rs:1");
    }

    #[test]
    fn supports_fixed_strings_and_type_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "a.b\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "a.b\n").unwrap();
        let backend = backend(dir.path());
        let mut opts = options("a.b");
        opts.fixed_strings = true;
        opts.file_type = Some("rust".to_owned());
        let text = backend.grep(&opts).unwrap();
        assert!(text.contains("a.rs"));
        assert!(!text.contains("b.txt"));
    }
}
