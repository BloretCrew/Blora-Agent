// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Slash command catalog and prefix / fuzzy completion.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub hint: &'static str,
    pub about: &'static str,
    pub group: &'static str,
}

const fn cmd(
    name: &'static str,
    aliases: &'static [&'static str],
    hint: &'static str,
    about: &'static str,
    group: &'static str,
) -> SlashCommand {
    SlashCommand {
        name,
        aliases,
        hint,
        about,
        group,
    }
}

/// Display order is the autocomplete order for a bare `/`.
pub const COMMANDS: &[SlashCommand] = &[
    cmd("help", &["?"], "[query]", "List slash commands", "system"),
    cmd("login", &[], "", "Sign in with Bloret PassPort", "system"),
    cmd("logout", &[], "", "Sign out of Bloret PassPort", "system"),
    cmd(
        "keymap",
        &["keys", "shortcuts"],
        "",
        "Show TUI key bindings",
        "system",
    ),
    cmd(
        "theme",
        &["appearance"],
        "[dark|light|auto]",
        "Show or set the TUI color theme (Blora Coral)",
        "system",
    ),
    cmd("new", &[], "", "Create a new session", "session"),
    cmd("sessions", &["ls-sessions"], "", "List sessions", "session"),
    cmd(
        "goto",
        &["open", "switch"],
        "<id|title>",
        "Switch to a session",
        "session",
    ),
    cmd(
        "status",
        &["info", "session-info"],
        "",
        "Show session id, mode, and run status",
        "session",
    ),
    cmd("id", &[], "", "Show the current session id", "session"),
    cmd(
        "context",
        &[],
        "",
        "Show tokens, sequence, and workspace",
        "session",
    ),
    cmd(
        "compact",
        &[],
        "[note]",
        "Compress conversation history",
        "run",
    ),
    cmd("checkpoint", &[], "", "Write a recovery checkpoint", "run"),
    cmd(
        "search",
        &["find-transcript"],
        "[query]",
        "Filter the transcript; empty query clears",
        "session",
    ),
    cmd(
        "find",
        &["grep"],
        "[query]",
        "Search session titles and transcripts",
        "session",
    ),
    cmd("clear", &[], "", "Clear transcript search", "session"),
    cmd(
        "tools",
        &[],
        "",
        "Toggle tool-call lines in the transcript",
        "session",
    ),
    cmd("copy", &[], "", "Copy the last assistant reply", "session"),
    cmd("cancel", &["stop"], "", "Cancel the in-flight run", "run"),
    cmd(
        "steer",
        &["interject"],
        "<message>",
        "Queue a message for the running turn (delivered at the next safe point)",
        "run",
    ),
    cmd(
        "yes",
        &["yolo", "always-approve", "auto"],
        "[on|off]",
        "Toggle or set auto-approve",
        "run",
    ),
    cmd("no", &[], "", "Turn auto-approve off", "run"),
    cmd(
        "permissions",
        &["policy"],
        "",
        "Show auto-approve, network, and isolation",
        "run",
    ),
    cmd("approvals", &[], "", "List pending approvals", "run"),
    cmd(
        "model",
        &["m"],
        "[name]",
        "Show or set the session model override",
        "run",
    ),
    cmd(
        "provider",
        &[],
        "[name]",
        "Show or set the session provider override",
        "run",
    ),
    cmd(
        "mode",
        &[],
        "[code|work|agent]",
        "Show or open a session in a mode",
        "session",
    ),
    cmd("code", &[], "", "Open a new code session", "session"),
    cmd("work", &[], "", "Open a new work session", "session"),
    cmd("agent", &[], "", "Open a new agent session", "session"),
    cmd(
        "plan",
        &[],
        "[prompt]",
        "Open an agent session titled plan",
        "session",
    ),
    cmd("exec", &["sandbox"], "", "Show BLORA_EXEC isolation", "run"),
    cmd("worktree", &[], "", "Show worktree execution flag", "run"),
    cmd("tasks", &[], "", "List background tasks", "work"),
    cmd(
        "task",
        &[],
        "<prompt>",
        "Queue a background task on this session",
        "work",
    ),
    cmd(
        "cron",
        &[],
        "<m h dom mon dow> <prompt>",
        "Queue a recurring cron task",
        "work",
    ),
    cmd(
        "loop",
        &[],
        "<duration> <prompt>",
        "Queue a delayed task (30s, 5m, 1h, 2d)",
        "work",
    ),
    cmd(
        "pause",
        &[],
        "<task-id>",
        "Pause a queued or scheduled task",
        "work",
    ),
    cmd(
        "unpause",
        &["resume-task"],
        "<task-id>",
        "Resume a paused task",
        "work",
    ),
    cmd(
        "cancel-task",
        &[],
        "<task-id>",
        "Cancel a background task",
        "work",
    ),
    cmd("pump", &[], "", "Run due background tasks once", "work"),
    cmd("fork", &[], "", "Fork this session", "session"),
    cmd("resume", &[], "", "Emit session.resumed", "session"),
    cmd("archive", &[], "", "Archive this session", "session"),
    cmd("export", &[], "", "Export the event log", "session"),
    cmd(
        "timeline",
        &["events"],
        "",
        "Show recent canonical events",
        "session",
    ),
    cmd("usage", &[], "", "Show recorded token usage", "session"),
    cmd("memory", &["mem"], "", "List stored memories", "memory"),
    cmd(
        "remember",
        &[],
        "<key> <value>",
        "Store a workspace memory",
        "memory",
    ),
    cmd(
        "recall",
        &[],
        "[key]",
        "Read a memory, or list them",
        "memory",
    ),
    cmd(
        "forget",
        &[],
        "<key>",
        "Delete a workspace memory",
        "memory",
    ),
    cmd(
        "distill",
        &["flush"],
        "",
        "Extract memories from this session",
        "memory",
    ),
    cmd(
        "plugins",
        &["plugin"],
        "",
        "List installed plugins",
        "plugins",
    ),
    cmd(
        "marketplace",
        &["market"],
        "",
        "List marketplace plugins",
        "plugins",
    ),
    cmd(
        "install",
        &[],
        "<name>",
        "Install a marketplace plugin",
        "plugins",
    ),
    cmd(
        "uninstall",
        &["remove"],
        "<name>",
        "Remove an installed plugin",
        "plugins",
    ),
    cmd("skills", &[], "", "List workspace skills", "plugins"),
    cmd("mcp", &["mcps"], "", "Show BLORA_MCP_COMMAND", "plugins"),
    cmd(
        "agents",
        &["subagents"],
        "",
        "List subagents for this session",
        "work",
    ),
    cmd("git", &[], "", "Show git status and branch", "git"),
    cmd("diff", &[], "", "Show git diff --stat", "git"),
    cmd("log", &[], "", "Show recent git log", "git"),
    cmd("files", &["ls"], "", "List workspace files", "git"),
    cmd("read", &[], "<path>", "Read a workspace file", "git"),
    cmd("artifacts", &[], "", "List session artifacts", "session"),
    cmd("pwd", &[], "", "Show workspace path", "session"),
    cmd(
        "doctor",
        &["settings"],
        "",
        "Show runtime environment flags",
        "system",
    ),
    cmd("hooks", &[], "", "Show BLORA_HOOKS_DIR", "plugins"),
    cmd(
        "init",
        &[],
        "",
        "Write .blora/rules.md if missing",
        "system",
    ),
    cmd("users", &[], "", "List gateway users", "system"),
    cmd("web", &[], "", "Show how to open the Web UI", "system"),
    cmd("reload", &[], "", "Reload the session list", "session"),
    cmd("next", &[], "", "Switch to the next session", "session"),
    cmd("prev", &[], "", "Switch to the previous session", "session"),
    cmd("quit", &["q", "exit"], "", "Leave the TUI", "system"),
];

pub const MAX_VISIBLE: usize = 10;

const GROUPS: &[&str] = &[
    "session", "run", "work", "memory", "plugins", "git", "system",
];

#[must_use]
pub fn is_open(input: &str) -> bool {
    input.starts_with('/') && !input.contains(' ')
}

#[must_use]
pub fn needs_args(cmd: &SlashCommand) -> bool {
    cmd.hint.starts_with('<')
}

#[must_use]
pub fn command_token(input: &str) -> Option<&str> {
    let rest = input.strip_prefix('/')?;
    Some(rest.split_whitespace().next().unwrap_or(""))
}

#[must_use]
pub fn matches(input: &str) -> Vec<&'static SlashCommand> {
    let Some(token) = command_token(input) else {
        return Vec::new();
    };
    let token = token.to_ascii_lowercase();
    if token.is_empty() {
        return COMMANDS.iter().collect();
    }
    let mut out: Vec<(&SlashCommand, u8)> = COMMANDS
        .iter()
        .filter_map(|cmd| rank(cmd, &token).map(|score| (cmd, score)))
        .collect();
    out.sort_by_key(|(cmd, score)| (*score, cmd.name.len(), cmd.name));
    out.into_iter().map(|(cmd, _)| cmd).collect()
}

fn rank(cmd: &SlashCommand, token: &str) -> Option<u8> {
    std::iter::once(cmd.name)
        .chain(cmd.aliases.iter().copied())
        .filter_map(|name| rank_name(name, token))
        .min()
}

fn rank_name(name: &str, token: &str) -> Option<u8> {
    if name == token {
        return Some(0);
    }
    if name.starts_with(token) {
        return Some(1);
    }
    if name.contains(token) {
        return Some(2);
    }
    if subsequence(name, token) {
        return Some(3);
    }
    None
}

fn subsequence(name: &str, token: &str) -> bool {
    let mut chars = name.chars();
    token.chars().all(|needle| chars.any(|ch| ch == needle))
}

#[must_use]
pub fn resolve(name: &str) -> Option<&'static SlashCommand> {
    let name = name.trim().trim_start_matches('/').to_ascii_lowercase();
    COMMANDS
        .iter()
        .find(|cmd| cmd.name == name || cmd.aliases.iter().any(|alias| *alias == name))
}

#[must_use]
pub fn complete(cmd: &SlashCommand) -> String {
    if cmd.hint.is_empty() {
        format!("/{}", cmd.name)
    } else {
        format!("/{} ", cmd.name)
    }
}

#[must_use]
pub fn help_text(filter: &str) -> String {
    let filter = filter.trim().to_ascii_lowercase();
    let mut lines = Vec::new();
    for group in GROUPS {
        let rows: Vec<_> = COMMANDS
            .iter()
            .filter(|cmd| cmd.group == *group)
            .filter(|cmd| {
                filter.is_empty()
                    || cmd.name.contains(&filter)
                    || cmd.about.to_ascii_lowercase().contains(&filter)
                    || cmd.aliases.iter().any(|alias| alias.contains(&filter))
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        lines.push(format!("[{group}]"));
        for cmd in rows {
            let alias = if cmd.aliases.is_empty() {
                String::new()
            } else {
                format!(" ({})", cmd.aliases.join(", "))
            };
            let hint = if cmd.hint.is_empty() {
                String::new()
            } else {
                format!(" {}", cmd.hint)
            };
            lines.push(format!("  /{}{hint}{alias}  {}", cmd.name, cmd.about));
        }
    }
    if lines.is_empty() {
        format!("no commands matching {filter}")
    } else {
        lines.join("\n")
    }
}

#[must_use]
pub fn keymap_text() -> String {
    [
        "Enter  send prompt, or run the selected slash command",
        "Tab    complete the selected slash command",
        "↑ / ↓  move in the slash menu (open after typing /)",
        "Ctrl+P open the slash menu",
        "Esc    close the slash menu or result panel; else quit",
        "Backspace  delete input",
        "[ / Left   previous session when the prompt is empty",
        "] / Right  next session when the prompt is empty",
        "PageUp / PageDown  scroll the transcript",
        "Home / End  oldest / newest transcript",
        "y / n  approve or deny when the prompt is empty",
        "Ctrl+N new session",
        "Ctrl+C request cancel and quit",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_slash_keeps_catalog_order() {
        let found = matches("/");
        assert_eq!(found.len(), COMMANDS.len());
        assert_eq!(found[0].name, "help");
        assert_eq!(found.last().map(|cmd| cmd.name), Some("quit"));
    }

    #[test]
    fn theme_command_is_catalogued() {
        assert_eq!(resolve("theme").map(|cmd| cmd.name), Some("theme"));
        assert_eq!(resolve("appearance").map(|cmd| cmd.name), Some("theme"));
        assert!(matches("/th").iter().any(|cmd| cmd.name == "theme"));
    }

    #[test]
    fn prefix_filters_and_ranks_exact() {
        let found = matches("/co");
        let names: Vec<_> = found.iter().map(|cmd| cmd.name).collect();
        assert!(names.contains(&"compact"));
        assert!(names.contains(&"copy"));
        assert!(names.contains(&"context"));
        let exact = matches("/yes");
        assert_eq!(exact[0].name, "yes");
    }

    #[test]
    fn fuzzy_subsequence_matches() {
        let found = matches("/cpt");
        assert!(found.iter().any(|cmd| cmd.name == "compact"));
        let found = matches("/gto");
        assert_eq!(found[0].name, "goto");
    }

    #[test]
    fn aliases_are_discoverable() {
        let found = matches("/yolo");
        assert_eq!(found[0].name, "yes");
        let found = matches("/q");
        assert_eq!(found[0].name, "quit");
        let found = matches("/?");
        assert_eq!(found[0].name, "help");
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = COMMANDS.iter().map(|cmd| cmd.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), COMMANDS.len());
    }

    #[test]
    fn complete_adds_space_when_hinted() {
        let task = resolve("task").unwrap();
        assert_eq!(complete(task), "/task ");
        assert!(needs_args(task));
        let quit = resolve("quit").unwrap();
        assert_eq!(complete(quit), "/quit");
        assert!(!needs_args(quit));
        let compact = resolve("compact").unwrap();
        assert!(!needs_args(compact));
        assert_eq!(complete(compact), "/compact ");
    }

    #[test]
    fn help_lists_groups_and_filters() {
        let all = help_text("");
        assert!(all.contains("[session]"));
        assert!(all.contains("/quit"));
        let git = help_text("git");
        assert!(git.contains("/diff"));
        assert!(!git.contains("/quit"));
    }

    #[test]
    fn is_open_until_args() {
        assert!(is_open("/"));
        assert!(is_open("/he"));
        assert!(!is_open("/task foo"));
        assert!(!is_open("hello"));
    }
}
