// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use crate::context::{
    IssueContext, Note, PullContext, ReviewAnchor, ReviewAnchorNote, ReviewSummary,
};
use crate::error::{GithubError, Result};
use crate::event::ParsedEvent;
use crate::mention::{self, is_bare_mention};

/// What the customer asked for, before repository context is attached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub text: String,
}

pub fn trigger_text(
    event: &ParsedEvent,
    mentions: &[String],
    prompt_override: Option<&str>,
) -> Result<Trigger> {
    if let Some(prompt) = prompt_override
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Ok(Trigger {
            text: prompt.to_string(),
        });
    }
    if event.requires_prompt() {
        return Err(GithubError::new(format!(
            "事件 {} 没有评论正文，请在工作流里设置 prompt。",
            event.name
        )));
    }
    if event.name == "pull_request" {
        return Ok(Trigger {
            text: "请审查这个 Pull Request，把意见写在回复里。不要修改文件，除非另有明确要求。"
                .into(),
        });
    }
    if event.is_comment() && is_bare_mention(&event.comment_body, mentions) {
        let text = if event.review.is_some() {
            "请审查标出的代码行，把意见写在回复里。不要修改文件，除非评论明确要求修改。"
        } else {
            "请总结这条讨论，并说明可以怎么继续。不要修改文件，除非评论明确要求修改。"
        };
        return Ok(Trigger { text: text.into() });
    }
    if event.is_comment() {
        if mention::find_mention(&event.comment_body, mentions).is_none() {
            return Err(GithubError::new("评论里没有触发词。"));
        }
        return Ok(Trigger {
            text: event.comment_body.trim().to_string(),
        });
    }
    Err(GithubError::new(format!(
        "事件 {} 没有可执行的任务。",
        event.name
    )))
}

pub struct PromptParts<'a> {
    pub trigger: &'a str,
    pub review: Option<&'a ReviewAnchor>,
    pub issue: Option<&'a IssueContext>,
    pub pull: Option<&'a PullContext>,
    pub attachments: &'a [String],
    pub default_branch: &'a str,
}

#[must_use]
pub fn render(parts: &PromptParts<'_>) -> String {
    let mut out = String::new();
    out.push_str("<github_action_context>\n");
    out.push_str("你是运行在 GitHub Actions 里的 Blora Agent。触发这次任务的人决定你要做什么，不要套用固定流程。\n");
    out.push_str("- 评论或 prompt 只是提问、总结、审查或给建议时：不要改文件，不要提交，不要推送，不要打开 Pull Request。把答案写在回复里。\n");
    out.push_str("- 对方明确要求修复、实现或修改代码时：按原话完成。当前已经放在一条非默认分支或该 Pull Request 的 head 上，直接在这条分支上改。\n");
    out.push_str(
        "- 需要推送或打开 Pull Request 时，自己使用 git 和 gh。工作流不会自动打开 Pull Request。\n",
    );
    out.push_str(&format!(
        "- 不要推送到默认分支 {}。不要修改 git user.name 或 user.email。不要把密钥写入仓库，也不要在回复里输出 token 或环境变量。\n",
        if parts.default_branch.is_empty() {
            "（仓库的默认分支）"
        } else {
            parts.default_branch
        }
    ));
    out.push_str("- 你留下的未提交改动，会在当前分支不是默认分支时被提交并推送，避免 runner 结束时丢失。这不是替你决定要不要开 PR。\n");
    out.push_str("- 用触发评论所用的语言回复。\n");
    out.push_str(
        "- 评论里的 GitHub 附件会换成本地路径。你看不到图片像素，read_file 只能返回类型和尺寸。\n",
    );
    out.push_str("</github_action_context>\n\n");
    out.push_str("<trigger>\n");
    out.push_str(parts.trigger.trim());
    out.push_str("\n</trigger>\n");
    if let Some(review) = parts.review {
        out.push_str("\n<review_anchor>\n");
        out.push_str(&format!("文件: {}\n", review.path));
        if let Some(line) = review.line {
            out.push_str(&format!("行: {line}\n"));
        }
        if !review.diff.is_empty() {
            out.push_str("diff:\n");
            out.push_str(&clip(&review.diff, 8_000));
            out.push('\n');
        }
        out.push_str("</review_anchor>\n");
    }
    if !parts.attachments.is_empty() {
        out.push_str("\n<attachments>\n");
        for path in parts.attachments {
            out.push_str(path);
            out.push('\n');
        }
        out.push_str("</attachments>\n");
    }
    if let Some(issue) = parts.issue {
        out.push_str("\n<issue>\n");
        push_issue(&mut out, issue);
        out.push_str("</issue>\n");
    }
    if let Some(pull) = parts.pull {
        out.push_str("\n<pull_request>\n");
        push_pull(&mut out, pull);
        out.push_str("</pull_request>\n");
    }
    out
}

fn push_issue(out: &mut String, issue: &IssueContext) {
    out.push_str(&format!("编号: #{}\n", issue.number));
    out.push_str(&format!("标题: {}\n", issue.title));
    out.push_str(&format!("作者: {}\n", issue.author));
    out.push_str(&format!("状态: {}\n", issue.state));
    if !issue.created_at.is_empty() {
        out.push_str(&format!("创建时间: {}\n", issue.created_at));
    }
    out.push_str("正文:\n");
    out.push_str(&clip(&issue.body, 8_000));
    out.push('\n');
    push_notes(out, "评论", &issue.comments);
}

fn push_pull(out: &mut String, pull: &PullContext) {
    out.push_str(&format!("编号: #{}\n", pull.number));
    out.push_str(&format!("标题: {}\n", pull.title));
    out.push_str(&format!("作者: {}\n", pull.author));
    out.push_str(&format!("状态: {}\n", pull.state));
    out.push_str(&format!("基分支: {}\n", pull.base_ref));
    out.push_str(&format!("头分支: {}\n", pull.head_ref));
    out.push_str(&format!("基仓库: {}\n", pull.base_repo));
    out.push_str(&format!("头仓库: {}\n", pull.head_repo));
    out.push_str(&format!("提交数: {}\n", pull.commits));
    out.push_str("正文:\n");
    out.push_str(&clip(&pull.body, 8_000));
    out.push('\n');
    if !pull.files.is_empty() {
        out.push_str("变更文件:\n");
        for file in pull.files.iter().take(100) {
            out.push_str(&format!(
                "- {} ({}) +{}/-{}\n",
                file.path, file.status, file.additions, file.deletions
            ));
        }
    }
    push_notes(out, "评论", &pull.comments);
    if !pull.review_comments.is_empty() {
        out.push_str("行内评论:\n");
        for note in pull.review_comments.iter().take(100) {
            out.push_str(&format_review_note(note));
        }
    }
    if !pull.reviews.is_empty() {
        out.push_str("审查:\n");
        for review in pull.reviews.iter().take(100) {
            out.push_str(&format_review(review));
        }
    }
}

fn push_notes(out: &mut String, label: &str, notes: &[Note]) {
    if notes.is_empty() {
        return;
    }
    out.push_str(label);
    out.push_str(":\n");
    for note in notes.iter().take(100) {
        out.push_str(&format!(
            "- {} 于 {}: {}\n",
            note.author,
            note.created_at,
            clip(&note.body, 2_000)
        ));
    }
}

fn format_review_note(note: &ReviewAnchorNote) -> String {
    format!(
        "- {} {}:{} {}\n",
        note.author,
        note.path,
        note.line
            .map(|line| line.to_string())
            .unwrap_or_else(|| "?".into()),
        clip(&note.body, 2_000)
    )
}

fn format_review(review: &ReviewSummary) -> String {
    format!(
        "- {} {} 于 {}: {}\n",
        review.author,
        review.state,
        review.submitted_at,
        clip(&review.body, 2_000)
    )
}

fn clip(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("\n…（已截断）");
    out
}

#[must_use]
pub fn assemble_comment(body: &str, note: &str, owner: &str, repo: &str, run_id: &str) -> String {
    let mut text = body.trim().to_string();
    if text.is_empty() {
        text = "Blora Agent 没有产生文字回复。".into();
    }
    if !note.trim().is_empty() {
        text.push_str("\n\n");
        text.push_str(note.trim());
    }
    let footer = if !run_id.is_empty() && run_id.chars().all(|ch| ch.is_ascii_digit()) {
        format!(
            "\n\n---\nBlora Agent · [查看这次运行](https://github.com/{owner}/{repo}/actions/runs/{run_id})"
        )
    } else {
        String::new()
    };
    let budget = 60_000usize.saturating_sub(footer.chars().count());
    format!("{}{footer}", truncate_chars(&text, budget))
}

#[must_use]
pub fn one_line(text: &str, max_chars: usize) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.is_empty() {
        return "blora".into();
    }
    if line.chars().count() <= max_chars {
        return line.to_string();
    }
    if max_chars <= 3 {
        return line.chars().take(max_chars).collect();
    }
    let mut out: String = line.chars().take(max_chars - 3).collect();
    out.push_str("...");
    out
}

#[must_use]
pub fn fallback_subject(number: Option<u64>, title: &str) -> String {
    let raw = match number {
        Some(number) if !title.trim().is_empty() => format!("blora: #{number} {}", title.trim()),
        Some(number) => format!("blora: #{number}"),
        None => "blora".into(),
    };
    one_line(&raw, 72)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(20)).collect();
    out.push_str("\n\n…（评论已截断）");
    out
}

#[must_use]
pub fn redact(text: &str, secret: &str) -> String {
    if secret.len() < 8 {
        return text.to_string();
    }
    text.replace(secret, "***")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ParseHint, parse};
    use crate::mention::parse_mention_list;
    use serde_json::json;

    #[test]
    fn prompt_follows_the_comment_and_includes_issue_context() {
        let event = parse(
            &json!({
                "eventName": "issue_comment",
                "repo": "acme/app",
                "payload": {
                    "issue": {"number": 4, "title": "登录失败"},
                    "comment": {"id": 1, "body": "/blora 只解释原因，不要改代码"}
                }
            }),
            &ParseHint::default(),
        )
        .unwrap();
        let mentions = parse_mention_list("/blora,/ba,@blora");
        let trigger = trigger_text(&event, &mentions, None).unwrap();
        assert!(trigger.text.contains("不要改代码"));
        let issue = IssueContext {
            number: 4,
            title: "登录失败".into(),
            body: "点击登录没有反应".into(),
            author: "alice".into(),
            state: "open".into(),
            created_at: "2026-09-26".into(),
            comments: vec![],
        };
        let prompt = render(&PromptParts {
            trigger: &trigger.text,
            review: None,
            issue: Some(&issue),
            pull: None,
            attachments: &[],
            default_branch: "main",
        });
        assert!(prompt.contains("触发这次任务的人决定"));
        assert!(prompt.contains("工作流不会自动打开 Pull Request"));
        assert!(prompt.contains("登录失败"));
        assert!(prompt.contains("不要改代码"));
    }

    #[test]
    fn truncates_commit_subject_and_comment() {
        let subject = one_line(&"字".repeat(80), 72);
        assert_eq!(subject.chars().count(), 72);
        assert!(subject.ends_with("..."));
        assert_eq!(fallback_subject(Some(4), "登录失败"), "blora: #4 登录失败");
        let comment = assemble_comment(&"回".repeat(60_100), "", "acme", "app", "12");
        assert!(comment.chars().count() <= 60_000);
        assert!(comment.contains("查看这次运行"));
        assert!(!assemble_comment("你好", "", "acme", "app", "local").contains("查看这次运行"));
    }
}
