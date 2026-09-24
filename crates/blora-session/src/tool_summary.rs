// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Collapse consecutive tool calls into one activity sentence.

use std::collections::BTreeSet;

use crate::TranscriptItem;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Verb {
    Search,
    Read,
    Edit,
    List,
    Shell,
    Git,
    Commit,
    Task,
    Delegate,
    Other,
}

/// One-line Chinese summary of a consecutive tool run, e.g.
/// `搜索了 5 次，编辑了 23 个文件，提交了 79sk7e，更新了任务清单。`
#[must_use]
pub fn summarize_tool_run(items: &[&TranscriptItem], _running: bool) -> String {
    let mut order: Vec<Verb> = Vec::new();
    let mut search = 0usize;
    let mut reads = BTreeSet::new();
    let mut edits = BTreeSet::new();
    let mut lists = 0usize;
    let mut shells = 0usize;
    let mut gits = 0usize;
    let mut commits: Vec<String> = Vec::new();
    let mut tasks = 0usize;
    let mut delegates = 0usize;
    let mut others = 0usize;
    let mut failed = 0usize;
    let mut inflight = false;

    for item in items {
        let TranscriptItem::Tool {
            name,
            status,
            arguments,
            output,
            ..
        } = item
        else {
            continue;
        };
        if status == "failed" || status == "error" {
            failed += 1;
        }
        if status == "running" {
            inflight = true;
        }
        let verb = classify(name, arguments.as_deref(), output.as_deref());
        if !order.contains(&verb) {
            order.push(verb);
        }
        match verb {
            Verb::Search => search += 1,
            Verb::Read => {
                reads.insert(
                    arg_field(arguments.as_deref(), "path").unwrap_or_else(|| name.clone()),
                );
            }
            Verb::Edit => {
                edits.insert(
                    arg_field(arguments.as_deref(), "path").unwrap_or_else(|| name.clone()),
                );
            }
            Verb::List => lists += 1,
            Verb::Shell => shells += 1,
            Verb::Git => gits += 1,
            Verb::Commit => {
                if let Some(hash) = commit_hash(arguments.as_deref(), output.as_deref()) {
                    let short_hash: String = hash.chars().take(6).collect();
                    if !commits.iter().any(|item| item == &short_hash) {
                        commits.push(short_hash);
                    }
                } else {
                    shells += 1;
                    if !order.contains(&Verb::Shell) {
                        order.push(Verb::Shell);
                    }
                }
            }
            Verb::Task => tasks += 1,
            Verb::Delegate => delegates += 1,
            Verb::Other => others += 1,
        }
    }

    let past = !inflight;
    let mut parts = Vec::new();
    for verb in order {
        let part = match verb {
            Verb::Search if search > 0 => Some(count_phrase(past, "搜索", search, "次")),
            Verb::Read if !reads.is_empty() => {
                Some(count_phrase(past, "读取", reads.len(), "个文件"))
            }
            Verb::Edit if !edits.is_empty() => {
                Some(count_phrase(past, "编辑", edits.len(), "个文件"))
            }
            Verb::List if lists > 0 => Some(count_phrase(past, "列出", lists, "个目录")),
            Verb::Shell if shells > 0 => Some(count_phrase(past, "运行", shells, "条命令")),
            Verb::Git if gits > 0 => Some(count_phrase(past, "查看 Git 状态", gits, "次")),
            Verb::Commit => {
                if commits.is_empty() {
                    None
                } else if past {
                    Some(format!("提交了 {}", commits.join("、")))
                } else {
                    Some("正在提交".to_owned())
                }
            }
            Verb::Task if tasks > 0 => Some(if past {
                "更新了任务清单".to_owned()
            } else {
                "正在更新任务清单".to_owned()
            }),
            Verb::Delegate if delegates > 0 => {
                Some(count_phrase(past, "派发", delegates, "个子代理"))
            }
            Verb::Other if others > 0 => Some(count_phrase(past, "调用工具", others, "次")),
            _ => None,
        };
        if let Some(part) = part {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        parts.push(if past {
            "完成了工具调用".to_owned()
        } else {
            "正在调用工具".to_owned()
        });
    }
    let mut text = parts.join("，");
    if failed > 0 {
        text.push_str(&format!(" · {failed} 次失败"));
    }
    text.push('。');
    text
}

fn count_phrase(past: bool, verb: &str, n: usize, unit: &str) -> String {
    if past {
        format!("{verb}了 {n} {unit}")
    } else {
        format!("正在{verb}")
    }
}

fn classify(name: &str, arguments: Option<&str>, output: Option<&str>) -> Verb {
    match name {
        "search" | "grep" | "glob" => Verb::Search,
        "read_file" | "read" => Verb::Read,
        "write_file" | "apply_patch" | "edit" => Verb::Edit,
        "list_dir" => Verb::List,
        "schedule_task" | "todo" | "todo_write" => Verb::Task,
        "delegate" => Verb::Delegate,
        "git_status" | "git_diff" | "git_log" | "git_branch" | "git_worktree" => Verb::Git,
        "shell" | "bash" | "exec" => {
            if looks_like_commit(arguments, output) {
                Verb::Commit
            } else {
                Verb::Shell
            }
        }
        _ => Verb::Other,
    }
}

fn looks_like_commit(arguments: Option<&str>, output: Option<&str>) -> bool {
    let blob = format!("{} {}", arguments.unwrap_or(""), output.unwrap_or(""));
    blob.contains("git commit") || commit_hash(arguments, output).is_some()
}

fn commit_hash(arguments: Option<&str>, output: Option<&str>) -> Option<String> {
    for text in [output, arguments].into_iter().flatten() {
        if let Some(hash) = find_short_hash(text) {
            return Some(hash);
        }
    }
    None
}

fn find_short_hash(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_hexdigit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            let len = i - start;
            if (7..41).contains(&len) {
                return Some(text[start..i].to_owned());
            }
        } else {
            i += 1;
        }
    }
    None
}

fn arg_field(arguments: Option<&str>, key: &str) -> Option<String> {
    let raw = arguments?;
    let needle = format!("\"{key}\"");
    let rest = raw.split(&needle).nth(1)?;
    let rest = rest.trim_start_matches([' ', ':', '"']);
    let end = rest.find('"').unwrap_or(rest.len());
    let value = rest[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_types::EventId;

    fn tool(name: &str, args: &str, output: Option<&str>) -> TranscriptItem {
        TranscriptItem::Tool {
            name: name.to_owned(),
            status: "completed".to_owned(),
            event_id: EventId::generate(),
            arguments: Some(args.to_owned()),
            output: output.map(ToOwned::to_owned),
            call_id: None,
        }
    }

    #[test]
    fn collapses_consecutive_tools_like_activity_sentence() {
        let search = tool("search", "{\"pattern\":\"foo\"}", None);
        let search2 = tool("search", "{\"pattern\":\"bar\"}", None);
        let edit = tool("apply_patch", "{\"path\":\"a.rs\"}", None);
        let edit2 = tool("write_file", "{\"path\":\"b.rs\"}", None);
        let commit = tool(
            "shell",
            "{\"command\":\"git commit -m x\"}",
            Some("[main 79abcdef] msg"),
        );
        let todo = tool("schedule_task", "{\"title\":\"t\"}", None);
        let items: Vec<&TranscriptItem> = vec![&search, &search2, &edit, &edit2, &commit, &todo];
        let text = summarize_tool_run(&items, false);
        assert!(text.contains("搜索了 2 次"), "{text}");
        assert!(text.contains("编辑了 2 个文件"), "{text}");
        assert!(text.contains("提交了 79abcd"), "{text}");
        assert!(!text.contains("79abcdef"), "{text}");
        assert!(text.contains("更新了任务清单"), "{text}");
        assert!(text.ends_with("任务清单。"), "{text}");
    }

    #[test]
    fn requested_tools_are_not_reported_as_running() {
        let mut read = tool("git_status", "{}", None);
        if let TranscriptItem::Tool { status, .. } = &mut read {
            *status = "requested".to_owned();
        }
        let text = summarize_tool_run(&[&read], false);
        assert_eq!(text, "查看 Git 状态了 1 次。");
    }

    #[test]
    fn historical_tools_stay_past_even_while_another_run_is_active() {
        let read = tool("read_file", "{\"path\":\"README.md\"}", None);
        let items = vec![&read];
        let text = summarize_tool_run(&items, true);
        assert!(text.contains("读取了 1 个文件"), "{text}");
        assert!(!text.contains("正在"), "{text}");
    }
}
