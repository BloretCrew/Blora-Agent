// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use serde_json::{Value, json};

use crate::context::{
    FileChange, IssueContext, Note, PullContext, ReactionSite, ReviewAnchorNote, ReviewSummary,
};
use crate::error::{GithubError, Result};
use crate::http::{Http, HttpRequest};

pub struct GithubApi<'a, H> {
    http: &'a H,
    token: String,
    base: String,
}

impl<'a, H: Http> GithubApi<'a, H> {
    pub fn new(http: &'a H, token: impl Into<String>, base: impl Into<String>) -> Self {
        let base = base.into().trim_end_matches('/').to_string();
        Self {
            http,
            token: token.into(),
            base,
        }
    }

    pub fn permission(&self, owner: &str, repo: &str, user: &str) -> Result<String> {
        let value = self.get_json(&format!(
            "/repos/{}/{}/collaborators/{}/permission",
            seg(owner),
            seg(repo),
            seg(user)
        ))?;
        Ok(value
            .get("permission")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    pub fn default_branch(&self, owner: &str, repo: &str) -> Result<String> {
        let value = self.get_json(&format!("/repos/{}/{}", seg(owner), seg(repo)))?;
        Ok(value
            .get("default_branch")
            .and_then(Value::as_str)
            .unwrap_or("main")
            .to_string())
    }

    pub fn load_issue(&self, owner: &str, repo: &str, number: u64) -> Result<IssueContext> {
        let issue = self.get_json(&format!(
            "/repos/{}/{}/issues/{number}",
            seg(owner),
            seg(repo)
        ))?;
        let comments = self.get_json(&format!(
            "/repos/{}/{}/issues/{number}/comments?per_page=100",
            seg(owner),
            seg(repo)
        ))?;
        Ok(IssueContext {
            number,
            title: text(&issue, "title"),
            body: text(&issue, "body"),
            author: login(&issue),
            state: text(&issue, "state"),
            created_at: text(&issue, "created_at"),
            comments: notes(&comments),
        })
    }

    pub fn load_pull(&self, owner: &str, repo: &str, number: u64) -> Result<PullContext> {
        let root = format!("/repos/{}/{}", seg(owner), seg(repo));
        let pull = self.get_json(&format!("{root}/pulls/{number}"))?;
        let files = self.get_json(&format!("{root}/pulls/{number}/files?per_page=100"))?;
        let comments = self.get_json(&format!("{root}/issues/{number}/comments?per_page=100"))?;
        let review_comments =
            self.get_json(&format!("{root}/pulls/{number}/comments?per_page=100"))?;
        let reviews = self.get_json(&format!("{root}/pulls/{number}/reviews?per_page=100"))?;
        let base = pull.get("base");
        let head = pull.get("head");
        Ok(PullContext {
            number,
            title: text(&pull, "title"),
            body: text(&pull, "body"),
            author: login(&pull),
            state: text(&pull, "state"),
            created_at: text(&pull, "created_at"),
            base_ref: nested(base, "ref"),
            head_ref: nested(head, "ref"),
            base_repo: repo_name(base),
            head_repo: repo_name(head),
            commits: pull.get("commits").and_then(Value::as_u64).unwrap_or(0),
            files: file_changes(&files),
            comments: notes(&comments),
            review_comments: review_notes(&review_comments),
            reviews: review_summaries(&reviews),
        })
    }

    pub fn add_eyes(&self, owner: &str, repo: &str, site: &ReactionSite) -> Result<u64> {
        let value = self.post_json(
            &reaction_path(owner, repo, site),
            &json!({"content": "eyes"}),
        )?;
        value
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| GithubError::new("GitHub 没有返回 reaction id"))
    }

    pub fn delete_reaction(
        &self,
        owner: &str,
        repo: &str,
        site: &ReactionSite,
        reaction_id: u64,
    ) -> Result<()> {
        let path = format!(
            "{}/reactions/{reaction_id}",
            reaction_path(owner, repo, site)
        );
        self.delete(&path)
    }

    pub fn comment(&self, owner: &str, repo: &str, number: u64, body: &str) -> Result<()> {
        self.post_json(
            &format!(
                "/repos/{}/{}/issues/{number}/comments",
                seg(owner),
                seg(repo)
            ),
            &json!({"body": body}),
        )?;
        Ok(())
    }

    fn get_json(&self, path: &str) -> Result<Value> {
        let bytes = self.request("GET", path, None)?;
        parse_json(&bytes)
    }

    fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        let bytes = serde_json::to_vec(body)
            .map_err(|err| GithubError::new(format!("无法编码 GitHub 请求：{err}")))?;
        let response = self.request("POST", path, Some(bytes))?;
        if response.is_empty() {
            return Ok(Value::Null);
        }
        parse_json(&response)
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.request("DELETE", path, None)?;
        Ok(())
    }

    fn request(&self, method: &str, path: &str, body: Option<Vec<u8>>) -> Result<Vec<u8>> {
        let url = format!("{}{path}", self.base);
        let response = self.http.send(&HttpRequest {
            method,
            url: &url,
            token: &self.token,
            body: body.as_deref(),
        })?;
        if !(200..300).contains(&response.status) {
            let text = String::from_utf8_lossy(&response.body);
            let snippet: String = text.chars().take(300).collect();
            return Err(GithubError::new(format!(
                "GitHub API {method} {path} 返回 {}：{snippet}",
                response.status
            )));
        }
        Ok(response.body)
    }
}

fn reaction_path(owner: &str, repo: &str, site: &ReactionSite) -> String {
    let root = format!("/repos/{}/{}", seg(owner), seg(repo));
    match site {
        ReactionSite::IssueComment { id } => format!("{root}/issues/comments/{id}/reactions"),
        ReactionSite::ReviewComment { id } => format!("{root}/pulls/comments/{id}/reactions"),
        ReactionSite::Issue { number } => format!("{root}/issues/{number}/reactions"),
    }
}

fn parse_json(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes).map_err(|err| {
        let snippet = String::from_utf8_lossy(bytes);
        let snippet: String = snippet.chars().take(180).collect();
        GithubError::new(format!("GitHub 返回的 JSON 无法解析：{err} {snippet}"))
    })
}

fn notes(value: &Value) -> Vec<Note> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| Note {
            id: item.get("id").and_then(Value::as_u64).unwrap_or(0),
            author: login(item),
            body: text(item, "body"),
            created_at: text(item, "created_at"),
        })
        .collect()
}

fn file_changes(value: &Value) -> Vec<FileChange> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| FileChange {
            path: text(item, "filename"),
            status: text(item, "status"),
            additions: item.get("additions").and_then(Value::as_u64).unwrap_or(0),
            deletions: item.get("deletions").and_then(Value::as_u64).unwrap_or(0),
        })
        .collect()
}

fn review_notes(value: &Value) -> Vec<ReviewAnchorNote> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| ReviewAnchorNote {
            id: item.get("id").and_then(Value::as_u64).unwrap_or(0),
            author: login(item),
            body: text(item, "body"),
            path: text(item, "path"),
            line: item.get("line").and_then(Value::as_i64),
            created_at: text(item, "created_at"),
        })
        .collect()
}

fn review_summaries(value: &Value) -> Vec<ReviewSummary> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| ReviewSummary {
            author: login(item),
            state: text(item, "state"),
            body: text(item, "body"),
            submitted_at: text(item, "submitted_at"),
        })
        .collect()
}

fn login(value: &Value) -> String {
    value
        .get("user")
        .and_then(|user| user.get("login"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn nested(value: Option<&Value>, key: &str) -> String {
    value
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn repo_name(side: Option<&Value>) -> String {
    side.and_then(|side| side.get("repo"))
        .and_then(|repo| repo.get("full_name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn seg(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpRequest, HttpResponse};
    use std::cell::RefCell;

    struct FakeHttp {
        calls: RefCell<Vec<String>>,
    }

    impl Http for FakeHttp {
        fn send(&self, request: &HttpRequest<'_>) -> Result<HttpResponse> {
            self.calls.borrow_mut().push(request.url.to_string());
            let body = if request.url.ends_with("/repos/acme/app/issues/4") {
                r#"{"title":"登录失败","body":"正文","state":"open","user":{"login":"alice"},"created_at":"2026-09-26"}"#
            } else if request.url.contains("/comments") {
                "[]"
            } else {
                r#"{"permission":"write"}"#
            };
            Ok(HttpResponse {
                status: 200,
                body: body.as_bytes().to_vec(),
            })
        }
    }

    #[test]
    fn loads_issue_without_treating_comment_url_as_the_issue() {
        let http = FakeHttp {
            calls: RefCell::new(Vec::new()),
        };
        let api = GithubApi::new(&http, "token-value-123", "https://api.github.test");
        let issue = api.load_issue("acme", "app", 4).unwrap();
        assert_eq!(issue.title, "登录失败");
        assert_eq!(issue.author, "alice");
        let permission = api.permission("acme", "app", "alice").unwrap();
        assert_eq!(permission, "write");
        let _ = api;
    }
}
