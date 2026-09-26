// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Compile canonical events into provider-facing messages.
//!
//! Layout follows the prefix-stability rules shared by production harnesses:
//!
//! 1. `[stable]` system block: identity and safety rules. Byte-identical for the
//!    life of the process and flagged as a cache breakpoint.
//! 2. `[context]` system block: project rules, skills, and memories. Changes only
//!    when files on disk change; also a breakpoint.
//! 3. Transcript compiled from events. Old tool results are micro-compacted.
//! 4. `<environment_context>` user fragment (date, git snapshot) placed directly
//!    before the latest user input so it never invalidates the shared prefix.
//!
//! Dates, git status, and other volatile facts never enter the system blocks.

use blora_events::{EventEnvelope, KnownPayload};
use blora_model::{ChatMessage, ToolCall};
use blora_types::{BloraError, Result};

/// Environment facts captured once per run so they stay stable across turns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvSnapshot {
    pub workspace: String,
    pub date: String,
    pub os: String,
    pub git: Option<String>,
    /// Permission mode for this run (`plan`, `ask`, `auto-edit`, `yolo`).
    pub permission_mode: String,
    /// Workspace-relative shell directory remembered for this session.
    pub shell_cwd: String,
}

impl EnvSnapshot {
    /// Capture the workspace environment. Git output is bounded and best-effort.
    #[must_use]
    pub fn capture(workspace: &str) -> Self {
        Self::capture_with_mode(workspace, "ask")
    }

    #[must_use]
    pub fn capture_with_mode(workspace: &str, permission_mode: &str) -> Self {
        Self {
            workspace: workspace.to_owned(),
            date: chrono::Utc::now().date_naive().to_string(),
            os: std::env::consts::OS.to_owned(),
            git: git_snapshot(workspace),
            permission_mode: permission_mode.to_owned(),
            shell_cwd: ".".to_owned(),
        }
    }

    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::from("<environment_context>\n");
        out.push_str(&format!("Workspace: {}\n", self.workspace));
        out.push_str(&format!("OS: {}\n", self.os));
        out.push_str(&format!("Date: {}\n", self.date));
        if !self.permission_mode.is_empty() {
            out.push_str(&format!("Permission mode: {}\n", self.permission_mode));
            out.push_str(match self.permission_mode.as_str() {
                "plan" => {
                    "Plan mode is read-only: explore with read_file, list_dir, search, and inspection-only shell commands, then write the plan to .blora/plan.md (the only writable path) and finish with a summary of the plan. Do not attempt other edits.\n"
                }
                "ask" => "Writes and mutating shell commands will pause for user approval.\n",
                "auto-edit" => {
                    "File edits inside the workspace are pre-approved; mutating shell commands still pause for approval.\n"
                }
                _ => "Writes and shell commands are pre-approved; dangerous commands still pause for approval.\n",
            });
        }
        if !self.shell_cwd.is_empty() {
            out.push_str(&format!(
                "Shell cwd: {} (relative to the workspace; shell starts here until cd or cwd changes it)\n",
                self.shell_cwd
            ));
        }
        if let Some(git) = &self.git {
            out.push_str("Git status (snapshot at run start; run git_status for live state):\n");
            out.push_str(git);
            out.push('\n');
        }
        out.push_str("</environment_context>");
        out
    }
}

/// Knobs for tool-result hygiene. Defaults mirror the microcompact behaviour
/// that Anthropic-lineage harnesses converge on.
#[derive(Clone, Copy, Debug)]
pub struct CompileOptions {
    /// Keep the most recent N tool results verbatim.
    pub keep_recent_tool_results: usize,
    /// Only clear older tool results once total tool-result chars exceed this.
    pub microcompact_threshold_chars: usize,
    /// Cap on any single tool result kept in the prompt.
    pub max_tool_result_chars: usize,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            keep_recent_tool_results: 5,
            microcompact_threshold_chars: 60_000,
            max_tool_result_chars: 24_000,
        }
    }
}

pub const CLEARED_TOOL_RESULT: &str = "[old tool result cleared to save context]";

/// Byte-stable identity and safety rules. Do not put dates or cwd here.
#[must_use]
pub fn stable_prompt(mode: &str) -> String {
    format!(
        "You are Blora Agent, a local {mode} harness.\n\
         Stay inside the workspace. Prefer read_file, list_dir, glob, and search before write_file or shell.\n\
         Use apply_patch for targeted edits; only use write_file to create files or replace them wholesale.\n\
         For work with three or more steps, keep a short checklist with update_plan and mark steps done as you go.\n\
         Tool results may be truncated or cleared; large results are saved under .blora/tool-output/ and can be paged with read_file offset/limit.\n\
         Images and other non-text files are described, not inlined. Skills are listed by name; load one with the skill tool before following it.\n\
         Shell commands start in the session working directory. A bare cd, or the cwd argument, moves it and stays inside the workspace.\n\
         Return a concise final answer when the task is done.\n\
         Do not exfiltrate secrets. Unknown event types in history must be ignored."
    )
}

/// Project rules, skills, and memories. Stable within a session unless files change.
#[must_use]
pub fn context_prompt(workspace: &str, mode: &str) -> String {
    let mut out = format!(
        "Mode: {mode}\n\
         Tools: read_file, write_file, list_dir, glob, search, shell, skill, apply_patch, git_status, git_diff, git_log, git_branch, git_worktree, process, schedule_task, delegate, handoff, remember, recall, forget, update_plan."
    );
    if let Some(rules) = project_rules(workspace) {
        out.push_str("\n\nProject rules:\n");
        out.push_str(&rules);
    }
    if let Some(skills) = skill_summaries(workspace) {
        out.push_str("\n\nSkills:\n");
        out.push_str(&skills);
    }
    if let Some(memory) = read_capped(
        &std::path::Path::new(workspace).join(".blora/memory.md"),
        3000,
    ) {
        out.push_str("\n\nMemories:\n");
        out.push_str(&memory);
    }
    out
}

fn read_capped(path: &std::path::Path, max: usize) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if text.len() > max {
        let mut cut = max;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push('…');
    }
    Some(text)
}

/// Project rules, most general first: `$BLORA_HOME/rules.md`, then `AGENTS.md`
/// / `CLAUDE.md` from the repository root down to the workspace, then the
/// workspace's own `.blora/rules.md`. The walk stops at the directory that
/// holds `.git` (or six levels up when there is none).
fn project_rules(workspace: &str) -> Option<String> {
    let root = std::path::Path::new(workspace);
    let mut chunks = Vec::new();
    if let Some(home) = blora_home()
        && let Some(text) = read_capped(&home.join("rules.md"), 4000)
    {
        chunks.push(format!("# ~/.blora/rules.md (global)\n{text}"));
    }
    for dir in rule_dirs(root) {
        let is_workspace = dir == root;
        for name in ["AGENTS.md", "CLAUDE.md"] {
            if let Some(text) = read_capped(&dir.join(name), 4000) {
                let label = if is_workspace {
                    name.to_owned()
                } else {
                    dir.join(name).display().to_string()
                };
                chunks.push(format!("# {label}\n{text}"));
            }
        }
        if is_workspace && let Some(text) = read_capped(&dir.join(".blora/rules.md"), 4000) {
            chunks.push(format!("# .blora/rules.md\n{text}"));
        }
    }
    if chunks.is_empty() {
        None
    } else {
        chunks.truncate(10);
        Some(chunks.join("\n\n"))
    }
}

/// Ancestor directories that may carry rules, outermost first, ending with the
/// workspace itself.
fn rule_dirs(workspace: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![workspace.to_path_buf()];
    let mut current = workspace;
    while dirs.len() < 6 && !current.join(".git").exists() {
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current || parent.as_os_str().is_empty() {
            break;
        }
        dirs.push(parent.to_path_buf());
        current = parent;
    }
    dirs.reverse();
    dirs
}

fn blora_home() -> Option<std::path::PathBuf> {
    if let Some(home) = std::env::var_os("BLORA_HOME") {
        return Some(std::path::PathBuf::from(home));
    }
    dirs::home_dir().map(|home| home.join(".blora"))
}

struct SkillFile {
    name: String,
    path: std::path::PathBuf,
}

fn discover_skills(workspace: &str) -> Vec<SkillFile> {
    let dir = std::path::Path::new(workspace).join(".blora/skills");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found = std::collections::BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            if let Some(name) = path.file_stem().and_then(|stem| stem.to_str())
                && skill_name_ok(name)
            {
                found.insert(name.to_owned(), path);
            }
        } else if path.is_dir()
            && let Some(name) = path.file_name().and_then(|name| name.to_str())
            && skill_name_ok(name)
        {
            let nested = path.join("SKILL.md");
            if nested.is_file() {
                found.entry(name.to_owned()).or_insert(nested);
            }
        }
    }
    found
        .into_iter()
        .map(|(name, path)| SkillFile { name, path })
        .collect()
}

fn skill_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphanumeric() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        && name.len() <= 64
}

fn skill_blurb(text: &str) -> String {
    let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return "(no description)".to_owned();
    };
    let line = line.trim_start_matches('#').trim();
    let mut blurb: String = line.chars().take(120).collect();
    if line.chars().count() > 120 {
        blurb.push('…');
    }
    if blurb.is_empty() {
        "(no description)".to_owned()
    } else {
        blurb
    }
}

fn skill_summaries(workspace: &str) -> Option<String> {
    let found = discover_skills(workspace);
    if found.is_empty() {
        return None;
    }
    let mut lines =
        vec!["Names only; call the skill tool with the name before following a skill.".to_owned()];
    for skill in found.into_iter().take(32) {
        let blurb = std::fs::read_to_string(&skill.path)
            .ok()
            .map(|text| skill_blurb(&text))
            .unwrap_or_else(|| "(unreadable)".to_owned());
        lines.push(format!("- {}: {blurb}", skill.name));
    }
    Some(lines.join("\n"))
}

/// Full text of one skill. The prompt lists names; this is the on-demand load.
pub fn read_skill(workspace: &str, name: &str) -> Result<String> {
    let name = name.trim();
    if !skill_name_ok(name) {
        return Err(BloraError::Other(format!(
            "invalid skill name {name}; use the name shown in the prompt"
        )));
    }
    let found = discover_skills(workspace);
    let Some(skill) = found.iter().find(|skill| skill.name == name) else {
        let names = found
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(BloraError::Other(format!(
            "unknown skill {name}; available: {}",
            if names.is_empty() { "(none)" } else { &names }
        )));
    };
    let mut text = std::fs::read_to_string(&skill.path).map_err(BloraError::exec)?;
    const MAX: usize = 16_000;
    if text.len() > MAX {
        let mut cut = MAX;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push('…');
    }
    Ok(text)
}

fn git_snapshot(workspace: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["status", "--short", "--branch"])
        .current_dir(workspace)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if text.is_empty() {
        None
    } else {
        Some(text.chars().take(2000).collect())
    }
}

/// Legacy single-string prompt kept for callers that want a preview.
#[must_use]
pub fn layered_system_prompt(workspace: &str, mode: &str) -> String {
    format!(
        "[stable]\n{}\n\n[context]\n{}\n\n[volatile]\n{}",
        stable_prompt(mode),
        context_prompt(workspace, mode),
        EnvSnapshot::capture(workspace).render()
    )
}

/// Rough token estimate: 4 chars per token, CJK-heavy text weighted up.
#[must_use]
pub fn estimate_tokens(text: &str) -> u64 {
    let chars = text.chars().count() as u64;
    let cjk = text
        .chars()
        .filter(|ch| matches!(*ch as u32, 0x3000..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF))
        .count() as u64;
    let base = chars.div_ceil(4);
    if cjk * 4 > chars {
        base + base / 2
    } else {
        base
    }
}

#[must_use]
pub fn estimate_messages(messages: &[ChatMessage]) -> u64 {
    messages
        .iter()
        .map(|message| {
            let mut total = 4 + estimate_tokens(message.content.as_deref().unwrap_or(""));
            if let Some(calls) = &message.tool_calls {
                for call in calls {
                    total += 8 + estimate_tokens(&call.name) + estimate_tokens(&call.arguments);
                }
            }
            total
        })
        .sum()
}

/// Context window used for compaction decisions. Override with `BLORA_CONTEXT_WINDOW`.
#[must_use]
pub fn context_window() -> u64 {
    std::env::var("BLORA_CONTEXT_WINDOW")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(128_000)
}

/// Auto-compaction threshold as a percentage of the window (default 85).
#[must_use]
pub fn compact_threshold_pct() -> u64 {
    std::env::var("BLORA_COMPACT_PCT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0 && *value <= 100)
        .unwrap_or(85)
}

/// Events after the most recent compaction, plus the summary carried by that event.
pub struct CompactionSplit<'a> {
    pub summary: Option<String>,
    pub events: &'a [EventEnvelope],
}

#[must_use]
pub fn split_at_compaction(events: &[EventEnvelope]) -> CompactionSplit<'_> {
    let Some(index) = events
        .iter()
        .rposition(|event| event.event_type == "context.compaction.completed")
    else {
        return CompactionSplit {
            summary: None,
            events,
        };
    };
    let (summary, preserve_from) = events[index]
        .decode_payload()
        .ok()
        .flatten()
        .and_then(|payload| match payload {
            KnownPayload::ContextCompactionCompleted(done) => {
                Some((done.summary, done.preserve_from_sequence))
            }
            _ => None,
        })
        .unwrap_or_else(|| ("(compacted)".to_owned(), None));
    // Preserve a verbatim tail: events at or after `preserve_from_sequence` that
    // precede the compaction event, followed by everything after it.
    let start = preserve_from
        .and_then(|seq| events.iter().position(|event| event.sequence >= seq))
        .filter(|start| *start <= index)
        .unwrap_or(index + 1);
    CompactionSplit {
        summary: Some(summary),
        events: &events[start..],
    }
}

/// Compile the transcript only (no system blocks). Used by compaction requests.
pub fn compile_transcript(events: &[EventEnvelope]) -> Result<Vec<ChatMessage>> {
    let mut messages = Vec::new();
    let mut assistant_text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    let flush_assistant = |messages: &mut Vec<ChatMessage>,
                           assistant_text: &mut String,
                           tool_calls: &mut Vec<ToolCall>| {
        if assistant_text.is_empty() && tool_calls.is_empty() {
            return;
        }
        let content = if assistant_text.is_empty() {
            None
        } else {
            Some(std::mem::take(assistant_text))
        };
        messages.push(ChatMessage::assistant(content, std::mem::take(tool_calls)));
    };

    for event in events {
        if event.event_type == "context.compaction.completed" {
            continue;
        }
        let Some(payload) = event.decode_payload()? else {
            continue;
        };
        match payload {
            KnownPayload::UserInput(input) => {
                flush_assistant(&mut messages, &mut assistant_text, &mut tool_calls);
                messages.push(ChatMessage::text("user", input.text));
            }
            KnownPayload::AssistantDelta(delta) => assistant_text.push_str(&delta.text),
            KnownPayload::AssistantMessageCompleted(completed) => {
                if assistant_text.is_empty() {
                    assistant_text = completed.text;
                }
            }
            KnownPayload::ToolRequested(requested) => {
                tool_calls.push(ToolCall {
                    id: requested
                        .call_id
                        .unwrap_or_else(|| format!("call_{}", requested.tool)),
                    name: requested.tool,
                    arguments: requested.arguments.to_string(),
                });
            }
            KnownPayload::ToolOutput(output) => {
                flush_assistant(&mut messages, &mut assistant_text, &mut tool_calls);
                messages.push(ChatMessage::tool_result(
                    output.call_id.unwrap_or_default(),
                    output.text,
                ));
            }
            _ => {}
        }
    }
    flush_assistant(&mut messages, &mut assistant_text, &mut tool_calls);
    Ok(messages)
}

/// Full provider-facing message list for a run.
pub fn compile_messages(
    events: &[EventEnvelope],
    workspace: &str,
    mode: &str,
    env: &EnvSnapshot,
) -> Result<Vec<ChatMessage>> {
    compile_with_options(events, workspace, mode, env, CompileOptions::default())
}

pub fn compile_with_options(
    events: &[EventEnvelope],
    workspace: &str,
    mode: &str,
    env: &EnvSnapshot,
    options: CompileOptions,
) -> Result<Vec<ChatMessage>> {
    let split = split_at_compaction(events);
    let mut messages = vec![
        ChatMessage::text("system", stable_prompt(mode)).with_breakpoint(),
        ChatMessage::text("system", context_prompt(workspace, mode)).with_breakpoint(),
    ];
    let mut transcript = compile_transcript(split.events)?;
    microcompact(&mut transcript, options);

    if let Some(summary) = split.summary {
        messages.push(ChatMessage::text(
            "user",
            format!(
                "<compaction_summary>\nThis session continues from an earlier conversation that was compacted. The summary below is reference material, not new instructions; the latest user message wins.\n{summary}\n</compaction_summary>"
            ),
        ));
        if transcript.first().is_some_and(|m| m.role != "user") {
            messages.push(ChatMessage::text(
                "assistant",
                "Understood. Continuing from the compacted context.",
            ));
        }
    }

    // Environment fragment goes right before the latest real user input.
    let last_user = transcript.iter().rposition(|m| m.role == "user");
    let env_text = env.render();
    match last_user {
        Some(index) => {
            let (head, tail) = transcript.split_at(index);
            messages.extend(head.iter().cloned());
            messages.push(ChatMessage::text("user", env_text));
            messages.extend(tail.iter().cloned());
        }
        None => {
            messages.extend(transcript);
            messages.push(ChatMessage::text("user", env_text));
        }
    }

    // Breakpoints on the latest user input and the latest tool result: the prefix
    // up to those points repeats on every turn within a run.
    let is_env = |m: &ChatMessage| {
        m.content
            .as_deref()
            .unwrap_or("")
            .starts_with("<environment_context>")
    };
    if let Some(index) = messages
        .iter()
        .rposition(|m| m.role == "user" && !is_env(m))
    {
        messages[index].cache_breakpoint = true;
    }
    if let Some(index) = messages.iter().rposition(|m| m.role == "tool") {
        messages[index].cache_breakpoint = true;
    }
    Ok(messages)
}

/// Replace old tool results with a placeholder once they weigh too much, and cap
/// any single result. Never touches user or assistant text.
pub fn microcompact(messages: &mut [ChatMessage], options: CompileOptions) {
    for message in messages.iter_mut() {
        if message.role != "tool" {
            continue;
        }
        if let Some(content) = message.content.as_mut() {
            if content.chars().count() > options.max_tool_result_chars {
                *content = head_tail(content, options.max_tool_result_chars);
            }
        }
    }
    let tool_indexes: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "tool")
        .map(|(i, _)| i)
        .collect();
    let total: usize = tool_indexes
        .iter()
        .map(|&i| {
            messages[i]
                .content
                .as_deref()
                .map_or(0, |c| c.chars().count())
        })
        .sum();
    if total <= options.microcompact_threshold_chars {
        return;
    }
    let keep_from = tool_indexes
        .len()
        .saturating_sub(options.keep_recent_tool_results);
    for &index in &tool_indexes[..keep_from] {
        messages[index].content = Some(CLEARED_TOOL_RESULT.to_owned());
    }
}

/// Keep the head and tail of a long text with a marker in between.
#[must_use]
pub fn head_tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let head_len = max_chars * 2 / 3;
    let tail_len = max_chars - head_len;
    let head: String = text.chars().take(head_len).collect();
    let tail: String = text.chars().skip(count.saturating_sub(tail_len)).collect();
    format!(
        "{head}\n\n[... {} chars truncated ...]\n\n{tail}",
        count - head_len - tail_len
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_events::{NewEvent, ToolOutput, UserInput};
    use blora_types::{SessionId, SystemClock};

    fn env() -> EnvSnapshot {
        EnvSnapshot {
            workspace: "/tmp".to_owned(),
            date: "2026-09-14".to_owned(),
            os: "linux".to_owned(),
            git: Some("## main".to_owned()),
            permission_mode: "plan".to_owned(),
            shell_cwd: ".".to_owned(),
        }
    }

    fn events(payloads: Vec<KnownPayload>) -> Vec<EventEnvelope> {
        let session = SessionId::generate();
        payloads
            .into_iter()
            .enumerate()
            .map(|(i, payload)| {
                EventEnvelope::from_new(
                    NewEvent::new(session.clone(), payload),
                    i as u64 + 1,
                    &SystemClock,
                )
                .unwrap()
            })
            .collect()
    }

    #[test]
    fn stable_prefix_does_not_include_volatile_facts() {
        let stable = stable_prompt("code");
        assert!(stable.contains("Blora Agent"));
        assert!(!stable.contains("Date:"));
        assert!(!context_prompt("/tmp", "code").contains("Date:"));
        assert!(env().render().contains("Date: 2026-09-14"));
        assert!(env().render().contains("Plan mode is read-only"));
    }

    #[test]
    fn includes_project_rules() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "prefer tests").unwrap();
        let prompt = context_prompt(dir.path().to_str().unwrap(), "code");
        assert!(prompt.contains("prefer tests"));
        assert!(prompt.contains("AGENTS.md"));
    }

    #[test]
    fn skills_are_listed_by_name_until_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join(".blora/skills");
        std::fs::create_dir_all(skills.join("review")).unwrap();
        std::fs::create_dir_all(skills.join("notes")).unwrap();
        std::fs::write(skills.join("review.md"), "# Review\nBODY-SHOULD-STAY-OUT\n").unwrap();
        std::fs::write(
            skills.join("review/SKILL.md"),
            "directory copy should lose to the file\n",
        )
        .unwrap();
        std::fs::write(skills.join("notes/SKILL.md"), "# Notes\nnested body\n").unwrap();
        let workspace = dir.path().to_str().unwrap();
        let prompt = context_prompt(workspace, "code");
        assert!(prompt.contains("- review: Review"));
        assert!(prompt.contains("- notes: Notes"));
        assert!(!prompt.contains("BODY-SHOULD-STAY-OUT"));
        assert!(!prompt.contains("nested body"));
        let loaded = read_skill(workspace, "notes").unwrap();
        assert!(loaded.contains("nested body"));
        assert!(read_skill(workspace, "../secret").is_err());
    }

    #[test]
    fn discovers_rules_up_to_the_repository_root() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let workspace = repo.join("crates").join("app");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "ABOVE-REPO").unwrap();
        std::fs::write(repo.join("AGENTS.md"), "REPO-WIDE").unwrap();
        std::fs::write(workspace.join("CLAUDE.md"), "LOCAL").unwrap();
        let prompt = context_prompt(workspace.to_str().unwrap(), "code");
        let repo_at = prompt.find("REPO-WIDE").expect("repo rules");
        let local_at = prompt.find("LOCAL").expect("workspace rules");
        assert!(repo_at < local_at, "outer rules come first");
        assert!(
            !prompt.contains("ABOVE-REPO"),
            "the walk stops at the .git root"
        );
    }

    #[test]
    fn environment_sits_before_latest_user_input_and_breakpoints_are_set() {
        let list = events(vec![
            KnownPayload::UserInput(UserInput {
                text: "first".to_owned(),
            }),
            KnownPayload::UserInput(UserInput {
                text: "second".to_owned(),
            }),
        ]);
        let messages = compile_messages(&list, "/tmp", "code", &env()).unwrap();
        assert_eq!(messages[0].role, "system");
        assert!(messages[0].cache_breakpoint);
        assert!(messages[1].cache_breakpoint);
        let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["system", "system", "user", "user", "user"]);
        assert!(
            messages[3]
                .content
                .as_deref()
                .unwrap()
                .starts_with("<environment_context>")
        );
        assert_eq!(messages[4].content.as_deref(), Some("second"));
        assert!(messages[4].cache_breakpoint);
    }

    #[test]
    fn microcompact_clears_old_tool_results_only() {
        let big = "x".repeat(30_000);
        let mut list = vec![ChatMessage::text("user", "hi")];
        for i in 0..7 {
            list.push(ChatMessage::assistant(
                None,
                vec![ToolCall {
                    id: format!("c{i}"),
                    name: "read_file".to_owned(),
                    arguments: "{}".to_owned(),
                }],
            ));
            list.push(ChatMessage::tool_result(format!("c{i}"), big.clone()));
        }
        microcompact(&mut list, CompileOptions::default());
        let cleared = list
            .iter()
            .filter(|m| m.content.as_deref() == Some(CLEARED_TOOL_RESULT))
            .count();
        assert_eq!(cleared, 2);
        assert_eq!(list[0].content.as_deref(), Some("hi"));
        let last = list.last().unwrap().content.as_deref().unwrap();
        assert!(last.contains("chars truncated"));
    }

    #[test]
    fn compaction_summary_keeps_verbatim_tail() {
        let mut list = events(vec![
            KnownPayload::UserInput(UserInput {
                text: "old".to_owned(),
            }),
            KnownPayload::UserInput(UserInput {
                text: "kept".to_owned(),
            }),
            KnownPayload::ToolOutput(ToolOutput {
                text: "ignored".to_owned(),
                call_id: None,
                truncated: false,
            }),
            KnownPayload::ContextCompactionCompleted(blora_events::ContextCompactionCompleted {
                summary: "SUMMARY".to_owned(),
                preserve_from_sequence: Some(2),
                tokens_before: 0,
                tokens_after: 0,
            }),
            KnownPayload::UserInput(UserInput {
                text: "new".to_owned(),
            }),
        ]);
        // sequences are 1..=5; preserve from 2 keeps "kept" and later.
        list[3].sequence = 4;
        let messages = compile_messages(&list, "/tmp", "code", &env()).unwrap();
        let texts: Vec<String> = messages.iter().filter_map(|m| m.content.clone()).collect();
        assert!(texts.iter().any(|t| t.contains("SUMMARY")));
        assert!(texts.iter().any(|t| t == "kept"));
        assert!(!texts.iter().any(|t| t == "old"));
        assert!(texts.iter().any(|t| t == "new"));
    }

    #[test]
    fn token_estimates_scale_with_text() {
        assert!(estimate_tokens("hello world") <= 4);
        assert!(estimate_tokens("你好世界你好世界") > estimate_tokens("abcdefgh"));
        assert_eq!(head_tail("abc", 10), "abc");
        assert!(head_tail(&"y".repeat(100), 30).contains("truncated"));
    }
}
