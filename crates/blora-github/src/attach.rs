// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::Path;

use regex::Regex;

use crate::error::{GithubError, Result};
use crate::http::{Http, HttpRequest};

/// Download GitHub user-attachments and replace their markdown with local paths.
pub fn rewrite<H: Http>(
    text: &str,
    http: &H,
    token: &str,
    dir: &Path,
) -> Result<(String, Vec<String>)> {
    let pattern = Regex::new(
        r#"(?i)(?:!\[[^\]]*\]\(|<img\b[^>]*\bsrc=["'])(https://github\.com/user-attachments/[^)\s"']+)"#,
    )
    .map_err(|err| GithubError::new(format!("附件表达式无效：{err}")))?;
    let mut hits: Vec<(usize, usize, String)> = pattern
        .captures_iter(text)
        .filter_map(|caps| {
            let whole = caps.get(0)?;
            let url = caps.get(1)?.as_str().to_string();
            Some((whole.start(), whole.end(), url))
        })
        .collect();
    hits.sort_by_key(|hit| hit.0);
    hits.dedup_by(|a, b| a.0 == b.0);
    if hits.is_empty() {
        return Ok((text.to_string(), Vec::new()));
    }
    std::fs::create_dir_all(dir)
        .map_err(|err| GithubError::new(format!("无法创建附件目录：{err}")))?;
    let mut saved = Vec::new();
    let mut output = text.to_string();
    for (index, (start, end, url)) in hits.into_iter().enumerate().rev() {
        let Some(path) = download(http, token, dir, &url, index)? else {
            continue;
        };
        let replacement = path.display().to_string();
        output.replace_range(start..end, &replacement);
        saved.push(replacement);
    }
    saved.reverse();
    Ok((output, saved))
}

fn download<H: Http>(
    http: &H,
    token: &str,
    dir: &Path,
    url: &str,
    index: usize,
) -> Result<Option<std::path::PathBuf>> {
    let response = http.send(&HttpRequest {
        method: "GET",
        url,
        token,
        body: None,
    })?;
    if !(200..300).contains(&response.status) || response.body.is_empty() {
        return Ok(None);
    }
    let name = url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("attachment");
    let name = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let path = dir.join(format!("{index}-{name}"));
    std::fs::write(&path, &response.body)
        .map_err(|err| GithubError::new(format!("无法保存附件：{err}")))?;
    Ok(Some(path))
}
