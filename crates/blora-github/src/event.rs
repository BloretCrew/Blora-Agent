// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use serde_json::Value;

use crate::context::ReviewAnchor;
use crate::error::{GithubError, Result};

/// Environment fields that accompany a raw webhook payload.
#[derive(Clone, Debug, Default)]
pub struct ParseHint {
    pub event_name: Option<String>,
    pub repository: Option<String>,
    pub actor: Option<String>,
}

/// Normalized GitHub event. The payload is context; it does not choose the outcome.
#[derive(Clone, Debug)]
pub struct ParsedEvent {
    pub name: String,
    pub owner: String,
    pub repo: String,
    pub actor: Option<String>,
    pub number: Option<u64>,
    pub is_pull_request: bool,
    pub comment_id: Option<u64>,
    pub comment_body: String,
    pub review: Option<ReviewAnchor>,
    pub issue_title: String,
    pub issue_body: String,
}

impl ParsedEvent {
    #[must_use]
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    #[must_use]
    pub fn is_comment(&self) -> bool {
        matches!(
            self.name.as_str(),
            "issue_comment" | "pull_request_review_comment"
        )
    }

    #[must_use]
    pub fn is_schedule(&self) -> bool {
        self.name == "schedule"
    }

    #[must_use]
    pub fn is_repo_automation(&self) -> bool {
        matches!(self.name.as_str(), "schedule" | "workflow_dispatch")
    }

    /// Events that have no comment body, so the workflow must pass `prompt`.
    #[must_use]
    pub fn requires_prompt(&self) -> bool {
        matches!(
            self.name.as_str(),
            "schedule" | "workflow_dispatch" | "issues"
        )
    }

    #[must_use]
    pub fn reaction_site(&self) -> Option<crate::context::ReactionSite> {
        if let Some(id) = self.comment_id {
            return Some(if self.name == "pull_request_review_comment" {
                crate::context::ReactionSite::ReviewComment { id }
            } else {
                crate::context::ReactionSite::IssueComment { id }
            });
        }
        self.number
            .map(|number| crate::context::ReactionSite::Issue { number })
    }
}

pub fn parse(raw: &Value, hint: &ParseHint) -> Result<ParsedEvent> {
    let wrapped = split_wrapper(raw);
    let wrapped_actor = wrapped.actor;
    let wrapped_repo = wrapped.repo;
    let payload = wrapped.payload;
    let name = wrapped
        .name
        .or_else(|| hint.event_name.clone())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            GithubError::new("缺少事件名。设置 GITHUB_EVENT_NAME，或在 JSON 里提供 eventName。")
        })?;
    if !supported(&name) {
        return Err(GithubError::new(format!("不支持的 GitHub 事件：{name}")));
    }
    let (owner, repo) = repository(hint, wrapped_repo.as_ref(), &payload)?;
    let actor = if name == "schedule" {
        None
    } else {
        wrapped_actor
            .or_else(|| hint.actor.clone())
            .or_else(|| login(payload.get("sender")))
            .or_else(|| login(payload.get("comment").and_then(|c| c.get("user"))))
            .filter(|value| !value.is_empty())
    };
    let comment = payload.get("comment");
    let comment_body = comment
        .and_then(|c| c.get("body"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let comment_id = comment.and_then(|c| json_u64(c.get("id")));
    let review = review_anchor(&name, comment);
    let (number, is_pull_request, issue_title, issue_body) = locate(&name, &payload);
    Ok(ParsedEvent {
        name,
        owner,
        repo,
        actor,
        number,
        is_pull_request,
        comment_id,
        comment_body,
        review,
        issue_title,
        issue_body,
    })
}

fn supported(name: &str) -> bool {
    matches!(
        name,
        "issue_comment"
            | "pull_request_review_comment"
            | "issues"
            | "pull_request"
            | "schedule"
            | "workflow_dispatch"
    )
}

struct WrappedEvent {
    name: Option<String>,
    actor: Option<String>,
    repo: Option<(String, String)>,
    payload: Value,
}

fn split_wrapper(raw: &Value) -> WrappedEvent {
    let Some(payload) = raw.get("payload") else {
        return WrappedEvent {
            name: None,
            actor: None,
            repo: None,
            payload: raw.clone(),
        };
    };
    let name = raw
        .get("eventName")
        .or_else(|| raw.get("event_name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    if name.is_none() {
        return WrappedEvent {
            name: None,
            actor: None,
            repo: None,
            payload: raw.clone(),
        };
    }
    WrappedEvent {
        name,
        actor: raw.get("actor").and_then(Value::as_str).map(str::to_owned),
        repo: repo_value(raw.get("repo")),
        payload: payload.clone(),
    }
}

fn repository(
    hint: &ParseHint,
    wrapped: Option<&(String, String)>,
    payload: &Value,
) -> Result<(String, String)> {
    if let Some(repo) = hint.repository.as_deref().and_then(split_full_name) {
        return Ok(repo);
    }
    if let Some(repo) = wrapped {
        return Ok(repo.clone());
    }
    if let Some(repo) = repo_value(payload.get("repository")) {
        return Ok(repo);
    }
    Err(GithubError::new(
        "缺少仓库坐标。设置 GITHUB_REPOSITORY，或在事件里提供 repository。",
    ))
}

fn repo_value(value: Option<&Value>) -> Option<(String, String)> {
    let value = value?;
    if let Some(text) = value.as_str() {
        return split_full_name(text);
    }
    if let (Some(owner), Some(repo)) = (
        value.get("owner").and_then(Value::as_str),
        value.get("repo").and_then(Value::as_str),
    ) {
        return Some((owner.to_string(), repo.to_string()));
    }
    let owner = value
        .get("owner")
        .and_then(|owner| owner.get("login"))
        .and_then(Value::as_str)
        .or_else(|| value.get("owner").and_then(Value::as_str))?;
    let name = value.get("name").and_then(Value::as_str).or_else(|| {
        value
            .get("full_name")
            .and_then(Value::as_str)
            .and_then(|full| full.split_once('/').map(|(_, repo)| repo))
    })?;
    Some((owner.to_string(), name.to_string()))
}

fn split_full_name(text: &str) -> Option<(String, String)> {
    let (owner, repo) = text.trim().trim_matches('/').split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

fn locate(name: &str, payload: &Value) -> (Option<u64>, bool, String, String) {
    match name {
        "pull_request_review_comment" | "pull_request" => {
            let pull = payload.get("pull_request");
            (
                pull.and_then(|pull| json_u64(pull.get("number"))),
                true,
                text(pull, "title"),
                text(pull, "body"),
            )
        }
        "issue_comment" | "issues" => {
            let issue = payload.get("issue");
            let is_pr = issue.and_then(|issue| issue.get("pull_request")).is_some();
            (
                issue.and_then(|issue| json_u64(issue.get("number"))),
                is_pr,
                text(issue, "title"),
                text(issue, "body"),
            )
        }
        _ => (None, false, String::new(), String::new()),
    }
}

fn review_anchor(name: &str, comment: Option<&Value>) -> Option<ReviewAnchor> {
    if name != "pull_request_review_comment" {
        return None;
    }
    let comment = comment?;
    Some(ReviewAnchor {
        path: text(Some(comment), "path"),
        line: comment.get("line").and_then(Value::as_i64),
        diff: text(Some(comment), "diff_hunk"),
    })
}

fn text(value: Option<&Value>, key: &str) -> String {
    value
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn login(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|value| value.get("login"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_wrapper_issue_and_marks_pull_request() {
        let issue = parse(
            &json!({
                "eventName": "issue_comment",
                "actor": "alice",
                "repo": {"owner": "acme", "repo": "app"},
                "payload": {
                    "issue": {"number": 4, "title": "登录失败", "body": "无法登录"},
                    "comment": {"id": 11, "body": "/blora 总结"}
                }
            }),
            &ParseHint::default(),
        )
        .unwrap();
        assert_eq!(issue.number, Some(4));
        assert!(!issue.is_pull_request);
        assert_eq!(issue.owner, "acme");
        assert_eq!(issue.comment_body, "/blora 总结");

        let pull = parse(
            &json!({
                "eventName": "issue_comment",
                "repo": "acme/app",
                "payload": {
                    "issue": {"number": 8, "pull_request": {"url": "https://example.test"}},
                    "comment": {"id": 2, "body": "@blora"}
                }
            }),
            &ParseHint::default(),
        )
        .unwrap();
        assert!(pull.is_pull_request);
        assert_eq!(pull.number, Some(8));
    }

    #[test]
    fn parses_raw_review_comment_with_hint() {
        let event = parse(
            &json!({
                "pull_request": {"number": 7, "title": "按钮"},
                "comment": {
                    "id": 3,
                    "body": "@blora 这里补上错误处理",
                    "path": "src/main.rs",
                    "line": 12,
                    "diff_hunk": "@@ -1 +1 @@\n-old\n+new"
                },
                "repository": {"name": "app", "owner": {"login": "acme"}},
                "sender": {"login": "alice"}
            }),
            &ParseHint {
                event_name: Some("pull_request_review_comment".into()),
                ..ParseHint::default()
            },
        )
        .unwrap();
        assert_eq!(event.review.as_ref().unwrap().path, "src/main.rs");
        assert_eq!(event.actor.as_deref(), Some("alice"));
        assert!(event.is_comment());
    }

    #[test]
    fn schedule_has_no_actor_and_rejects_unknown_events() {
        let event = parse(
            &json!({"eventName": "schedule", "repo": "acme/app", "payload": {"schedule": "0 0 * * *"}}),
            &ParseHint::default(),
        )
        .unwrap();
        assert!(event.actor.is_none());
        assert!(event.requires_prompt());
        assert!(event.is_schedule());
        let err = parse(
            &json!({"eventName": "push", "repo": "acme/app", "payload": {}}),
            &ParseHint::default(),
        );
        assert!(err.is_err());
    }
}
