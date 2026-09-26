// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Fail-closed workspace policy.
//!
//! A [`PermissionMode`] decides how much the agent may do without asking. Shell
//! commands are additionally classified by [`classify_shell`] so that read-only
//! inspection never nags and dangerous commands always confirm, whatever the mode.

/// Increment when capability defaults change. Recorded on approval events.
pub const POLICY_VERSION: u32 = 2;

use std::path::{Path, PathBuf};

use blora_types::{BloraError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

/// How much the agent may do without a human in the loop.
///
/// Ordered from least to most permissive; `Shift+Tab` in the TUI cycles through
/// them in this order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PermissionMode {
    /// Read-only exploration. The only writable path is `.blora/plan.md`;
    /// every mutating shell command is denied outright.
    Plan,
    /// Confirm every write and every shell command that is not read-only
    /// inspection.
    #[default]
    Ask,
    /// Workspace file edits are allowed without asking; shell commands that
    /// mutate anything still confirm.
    AutoEdit,
    /// Everything is allowed except dangerous commands and network access.
    Yolo,
}

impl PermissionMode {
    pub const ALL: [Self; 4] = [Self::Plan, Self::Ask, Self::AutoEdit, Self::Yolo];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Ask => "ask",
            Self::AutoEdit => "auto-edit",
            Self::Yolo => "yolo",
        }
    }

    /// Parse a user-facing name. Accepts the canonical names plus common aliases.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "plan" | "read-only" | "readonly" | "ro" => Some(Self::Plan),
            "ask" | "default" | "normal" | "confirm" => Some(Self::Ask),
            "auto-edit" | "autoedit" | "edit" | "accept-edits" | "edits" => Some(Self::AutoEdit),
            "yolo" | "auto" | "always-approve" | "bypass" | "yes" => Some(Self::Yolo),
            _ => None,
        }
    }

    /// The mode after this one in the `Shift+Tab` cycle (wraps around).
    #[must_use]
    pub fn cycle(self) -> Self {
        match self {
            Self::Plan => Self::Ask,
            Self::Ask => Self::AutoEdit,
            Self::AutoEdit => Self::Yolo,
            Self::Yolo => Self::Plan,
        }
    }

    /// True when non-interactive runs can proceed without approvals for
    /// ordinary writes and shell commands.
    #[must_use]
    pub fn auto_approve(self) -> bool {
        matches!(self, Self::Yolo)
    }

    /// Short human description shown in status lines and help.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Plan => "read-only; only .blora/plan.md is writable",
            Self::Ask => "confirm writes and mutating commands",
            Self::AutoEdit => "edits allowed; mutating commands confirm",
            Self::Yolo => "allow all except dangerous commands and network",
        }
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Coarse risk class of a shell command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellClass {
    /// Every segment is a known inspection command with no redirection.
    ReadOnly,
    /// Something may change files or processes but nothing looks destructive.
    Mutating,
    /// Patterns that are hard to undo: recursive deletes, privilege escalation,
    /// disk-level tools, force pushes, piping downloads into a shell.
    Dangerous,
}

/// The path that stays writable in [`PermissionMode::Plan`].
pub const PLAN_FILE: &str = ".blora/plan.md";

#[derive(Clone, Debug)]
pub struct Policy {
    workspace: PathBuf,
    mode: PermissionMode,
    /// Set by [`Policy::granting`] after the user approved an action: every
    /// `Ask` becomes `Allow` for the retried call. `Deny` stays `Deny`.
    granted: bool,
}

impl Policy {
    /// Compatibility constructor: `auto_approve` maps to [`PermissionMode::Yolo`],
    /// otherwise [`PermissionMode::Ask`].
    pub fn new(workspace: impl Into<PathBuf>, auto_approve: bool) -> Result<Self> {
        Self::with_mode(
            workspace,
            if auto_approve {
                PermissionMode::Yolo
            } else {
                PermissionMode::Ask
            },
        )
    }

    pub fn with_mode(workspace: impl Into<PathBuf>, mode: PermissionMode) -> Result<Self> {
        let workspace = workspace.into();
        let workspace = workspace.canonicalize().unwrap_or(workspace);
        if !workspace.exists() {
            return Err(BloraError::Policy(format!(
                "workspace does not exist: {}",
                workspace.display()
            )));
        }
        Ok(Self {
            workspace,
            mode,
            granted: false,
        })
    }

    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    #[must_use]
    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    #[must_use]
    pub fn auto_approve(&self) -> bool {
        self.mode.auto_approve()
    }

    /// A copy that treats one approved action as granted. Used after the user
    /// says yes so the retried call does not ask again (even for dangerous
    /// commands, which keep asking under every mode).
    #[must_use]
    pub fn granting(&self) -> Self {
        Self {
            workspace: self.workspace.clone(),
            mode: self.mode,
            granted: true,
        }
    }

    pub fn resolve(&self, requested: &str) -> Result<PathBuf> {
        let requested = requested.trim();
        if requested.is_empty() || requested == "." {
            return Ok(self.workspace.clone());
        }
        let raw = PathBuf::from(requested);
        let candidate = if raw.is_absolute() {
            raw
        } else {
            self.workspace.join(raw)
        };
        let resolved = candidate
            .canonicalize()
            .unwrap_or_else(|_| normalize_path(&candidate));
        if !resolved.starts_with(&self.workspace) {
            return Err(BloraError::Policy(format!(
                "path escapes workspace: {requested}"
            )));
        }
        Ok(resolved)
    }

    #[must_use]
    pub fn file_read(&self) -> Decision {
        Decision::Allow
    }

    #[must_use]
    pub fn file_read_path(&self, path: &str) -> Decision {
        if looks_secret(path) {
            self.secrets()
        } else {
            Decision::Allow
        }
    }

    /// Generic write capability (worktree add, unknown targets).
    #[must_use]
    pub fn file_write(&self) -> Decision {
        match self.mode {
            PermissionMode::Plan => Decision::Deny,
            PermissionMode::Ask => Decision::Ask,
            PermissionMode::AutoEdit | PermissionMode::Yolo => Decision::Allow,
        }
    }

    /// Write capability for a specific workspace-relative path.
    #[must_use]
    pub fn file_write_path(&self, path: &str) -> Decision {
        if self.mode == PermissionMode::Plan {
            return if is_plan_file(path) {
                Decision::Allow
            } else {
                Decision::Deny
            };
        }
        if looks_secret(path) {
            return self.secrets();
        }
        self.file_write()
    }

    #[must_use]
    pub fn search(&self) -> Decision {
        Decision::Allow
    }

    /// Shell capability when the command text is not known (process kill).
    #[must_use]
    pub fn shell(&self) -> Decision {
        match self.mode {
            PermissionMode::Plan => Decision::Deny,
            PermissionMode::Ask | PermissionMode::AutoEdit => Decision::Ask,
            PermissionMode::Yolo => Decision::Allow,
        }
    }

    /// Decision for a concrete command line. Network commands defer to
    /// [`Policy::network`]; otherwise the [`ShellClass`] and the mode decide.
    #[must_use]
    pub fn shell_command(&self, command: &str) -> Decision {
        if looks_network(command) {
            return self.network();
        }
        match classify_shell(command) {
            ShellClass::Dangerous => match self.mode {
                PermissionMode::Plan => Decision::Deny,
                _ => Decision::Ask,
            },
            ShellClass::ReadOnly => match self.mode {
                PermissionMode::Ask => Decision::Ask,
                _ => Decision::Allow,
            },
            ShellClass::Mutating => {
                if looks_git_destructive(command) {
                    return self.git_destructive();
                }
                self.shell()
            }
        }
    }

    #[must_use]
    pub fn network(&self) -> Decision {
        if env_flag("BLORA_NETWORK") {
            match self.mode {
                PermissionMode::Plan => Decision::Deny,
                PermissionMode::Yolo => Decision::Allow,
                _ => Decision::Ask,
            }
        } else {
            Decision::Deny
        }
    }

    #[must_use]
    pub fn git_destructive(&self) -> Decision {
        match self.mode {
            PermissionMode::Plan => Decision::Deny,
            PermissionMode::Yolo => Decision::Allow,
            _ => Decision::Ask,
        }
    }

    #[must_use]
    pub fn secrets(&self) -> Decision {
        if self.mode == PermissionMode::Yolo {
            Decision::Allow
        } else {
            Decision::Ask
        }
    }

    pub fn require(&self, decision: Decision, summary: &str) -> Result<()> {
        match decision {
            Decision::Allow => Ok(()),
            Decision::Ask if self.granted => Ok(()),
            Decision::Deny => Err(BloraError::Policy(if self.mode == PermissionMode::Plan {
                format!("{summary}: denied in plan mode (read-only; only {PLAN_FILE} is writable)")
            } else {
                summary.to_owned()
            })),
            Decision::Ask => Err(BloraError::ApprovalRequired(summary.to_owned())),
        }
    }
}

fn is_plan_file(path: &str) -> bool {
    let normalized = path.trim().trim_start_matches("./").replace('\\', "/");
    normalized == PLAN_FILE || normalized.ends_with(&format!("/{PLAN_FILE}"))
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
    })
}

/// Commands that only inspect state. A segment whose first word is listed here
/// and that carries no redirection is read-only.
const READ_ONLY_BINS: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "ag",
    "find",
    "fd",
    "wc",
    "echo",
    "printf",
    "pwd",
    "stat",
    "file",
    "which",
    "whereis",
    "type",
    "env",
    "printenv",
    "date",
    "uname",
    "whoami",
    "id",
    "hostname",
    "uptime",
    "df",
    "du",
    "free",
    "ps",
    "pgrep",
    "tree",
    "diff",
    "cmp",
    "sort",
    "uniq",
    "cut",
    "tr",
    "awk",
    "sed",
    "jq",
    "yq",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "md5sum",
    "sha256sum",
    "sha1sum",
    "true",
    "false",
    "test",
    "[",
    "column",
    "nl",
    "od",
    "xxd",
    "hexdump",
    "strings",
    "tac",
    "rev",
    "seq",
    "expr",
    "bc",
    "tput",
    "cargo",
    "rustc",
    "npm",
    "pnpm",
    "yarn",
    "bun",
    "go",
    "git",
];

/// Subcommands that make an otherwise inspection-only binary mutate things.
fn subcommand_mutates(bin: &str, sub: &str) -> bool {
    match bin {
        "git" => !matches!(
            sub,
            "status"
                | "log"
                | "diff"
                | "show"
                | "blame"
                | "rev-parse"
                | "ls-files"
                | "describe"
                | "shortlog"
                | "grep"
                | "cat-file"
                | "reflog"
                | "--version"
                | "version"
                | "help"
                | ""
        ),
        "cargo" => !matches!(
            sub,
            "check"
                | "clippy"
                | "test"
                | "build"
                | "fmt"
                | "doc"
                | "tree"
                | "metadata"
                | "bench"
                | "run"
                | "--version"
                | "version"
                | "help"
                | ""
        ),
        "npm" | "pnpm" | "yarn" | "bun" => !matches!(
            sub,
            "test"
                | "run"
                | "ls"
                | "list"
                | "outdated"
                | "view"
                | "info"
                | "why"
                | "audit"
                | "--version"
                | "-v"
                | ""
        ),
        "go" => !matches!(
            sub,
            "test" | "build" | "vet" | "list" | "version" | "env" | ""
        ),
        "sed" => sub.starts_with("-i") || sub == "--in-place",
        _ => false,
    }
}

/// Patterns that are hard to undo. Matched against each segment after
/// whitespace normalisation, so `rm  -rf` and `rm -r -f` both count.
fn segment_dangerous(segment: &str) -> bool {
    let words: Vec<&str> = segment.split_whitespace().collect();
    let Some(&first) = words.first() else {
        return false;
    };
    let bin = base_name(first);
    let flags = words[1..]
        .iter()
        .filter(|w| w.starts_with('-'))
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let has_flag = |needle: char| {
        flags
            .iter()
            .any(|f| !f.starts_with("--") && f.contains(needle))
    };
    let long_flag = |needle: &str| flags.iter().any(|f| f == needle);
    match bin.as_str() {
        "rm" => has_flag('r') || long_flag("--recursive"),
        "sudo" | "su" | "doas" | "dd" | "mkfs" | "fdisk" | "parted" | "shutdown" | "reboot"
        | "halt" | "poweroff" | "shred" | "wipefs" | "systemctl" | "launchctl" | "kill"
        | "killall" | "pkill" | "chroot" | "mount" | "umount" | "iptables" | "nft" | "eval"
        | "exec" => true,
        "crontab" => words.get(1) != Some(&"-l"),
        "chmod" | "chown" | "chgrp" => has_flag('r') || long_flag("--recursive"),
        "git" => {
            let sub = words.get(1).copied().unwrap_or("");
            match sub {
                "push" => {
                    has_flag('f')
                        || long_flag("--force")
                        || long_flag("--force-with-lease")
                        || words.iter().any(|w| *w == "--delete" || *w == "-d")
                }
                "reset" => long_flag("--hard"),
                "checkout" => has_flag('f') || long_flag("--force"),
                "restore" => words.contains(&"."),
                "branch" => has_flag('d') || long_flag("--delete"),
                "clean" | "filter-branch" | "filter-repo" | "gc" | "prune" => true,
                _ => false,
            }
        }
        "truncate" => words.iter().any(|w| *w == "-s" || w.starts_with("--size")),
        "find" => words.iter().any(|w| *w == "-delete" || *w == "-exec"),
        "xargs" => words.iter().any(|w| base_name(w) == "rm"),
        _ => false,
    }
}

fn base_name(word: &str) -> String {
    word.rsplit('/').next().unwrap_or(word).to_ascii_lowercase()
}

/// Split a command line into simple segments at `&&`, `||`, `;`, `|`, `&` and
/// newlines. Quotes are respected so `grep "a;b"` stays one segment, and the
/// `&` of `2>&1` / `&>` is a redirection, not a separator.
fn split_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some(q) => {
                current.push(ch);
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    current.push(ch);
                }
                '\\' => {
                    current.push(ch);
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                '&' if current.ends_with('>') || chars.peek() == Some(&'>') => current.push(ch),
                '&' | '|' | ';' | '\n' => {
                    if ch != '\n' && chars.peek() == Some(&ch) {
                        chars.next();
                    }
                    if !current.trim().is_empty() {
                        segments.push(current.trim().to_owned());
                    }
                    current.clear();
                }
                _ => current.push(ch),
            },
        }
    }
    if !current.trim().is_empty() {
        segments.push(current.trim().to_owned());
    }
    segments
}

/// Classify a full command line. Any dangerous segment makes the whole line
/// dangerous; any non-read-only segment makes it mutating.
#[must_use]
pub fn classify_shell(command: &str) -> ShellClass {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return ShellClass::ReadOnly;
    }
    let lower = trimmed.to_ascii_lowercase();
    // Piping a download into an interpreter, command substitution with rm,
    // fork bombs, and writes to devices are dangerous regardless of segments.
    let pipes_into_shell = [
        "| sh",
        "| bash",
        "|sh",
        "|bash",
        "| zsh",
        "|zsh",
        "| cmd",
        "|cmd",
        "| powershell",
        "|powershell",
        "| pwsh",
        "|pwsh",
    ]
    .iter()
    .any(|needle| lower.contains(*needle));
    if pipes_into_shell
        || lower.contains("> /dev/sd")
        || lower.contains("> /dev/nvme")
        || lower.contains(":(){")
        || ((lower.contains("$(") || lower.contains('`')) && lower.contains("rm "))
    {
        return ShellClass::Dangerous;
    }
    let segments = split_segments(trimmed);
    let mut class = ShellClass::ReadOnly;
    for segment in &segments {
        if segment_dangerous(segment) {
            return ShellClass::Dangerous;
        }
        let words: Vec<&str> = segment.split_whitespace().collect();
        let first = words.first().copied().unwrap_or("");
        let bin = base_name(first);
        let sub = words.get(1).copied().unwrap_or("");
        // stderr-to-stdout merges and stderr discards are not writes.
        let without_stderr = segment
            .replace("2>&1", "")
            .replace("2>/dev/null", "")
            .replace("2> /dev/null", "");
        let redirects = without_stderr.contains('>');
        let read_only = READ_ONLY_BINS.contains(&bin.as_str())
            && !subcommand_mutates(&bin, sub)
            && !redirects
            && !segment.contains(" -exec ")
            && !(bin == "awk" && segment.contains("system("));
        if !read_only {
            class = ShellClass::Mutating;
        }
    }
    class
}

fn first_bin(command: &str) -> String {
    base_name(command.split_whitespace().next().unwrap_or(""))
}

fn looks_network(command: &str) -> bool {
    split_segments(command).iter().any(|segment| {
        matches!(
            first_bin(segment).as_str(),
            "curl" | "wget" | "nc" | "ncat" | "ssh" | "scp" | "ftp" | "aria2c" | "telnet" | "rsync"
        )
    })
}

fn looks_git_destructive(command: &str) -> bool {
    split_segments(command).iter().any(|segment| {
        let mut words = segment.split_whitespace();
        if first_bin(segment) != "git" {
            return false;
        }
        let sub = words.nth(1).unwrap_or("");
        matches!(
            sub,
            "push" | "commit" | "reset" | "clean" | "rebase" | "filter-branch" | "merge" | "am"
        )
    })
}

fn looks_secret(path: &str) -> bool {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
        .to_ascii_lowercase();
    name.contains(".env")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name == "id_rsa"
        || name == "id_ed25519"
        || name == "credentials"
        || name == "secrets.json"
}

fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(mode: PermissionMode) -> (tempfile::TempDir, Policy) {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::with_mode(dir.path(), mode).unwrap();
        (dir, policy)
    }

    #[test]
    fn rejects_path_escape() {
        let (_dir, policy) = policy(PermissionMode::Ask);
        assert!(policy.resolve("../etc/passwd").is_err());
        assert!(policy.resolve(".").is_ok());
    }

    #[test]
    fn denies_network_by_default() {
        let (_dir, policy) = policy(PermissionMode::Ask);
        assert_eq!(policy.network(), Decision::Deny);
        assert_eq!(
            policy.shell_command("curl https://example.test"),
            Decision::Deny
        );
        assert_eq!(policy.file_read_path(".env"), Decision::Ask);
        assert_eq!(policy.git_destructive(), Decision::Ask);
    }

    #[test]
    fn classifies_shell_commands() {
        assert_eq!(classify_shell("ls -la"), ShellClass::ReadOnly);
        assert_eq!(
            classify_shell("git status && git diff"),
            ShellClass::ReadOnly
        );
        assert_eq!(
            classify_shell("cargo test -p blora-policy"),
            ShellClass::ReadOnly
        );
        assert_eq!(
            classify_shell("grep -rn \"a;b\" src | head"),
            ShellClass::ReadOnly
        );
        assert_eq!(classify_shell("echo hi > out.txt"), ShellClass::Mutating);
        assert_eq!(
            classify_shell("cargo test 2>&1 | tail"),
            ShellClass::ReadOnly
        );
        assert_eq!(
            classify_shell("cargo build &> log.txt"),
            ShellClass::Mutating
        );
        assert_eq!(classify_shell("ls & sleep 1"), ShellClass::Mutating);
        assert_eq!(classify_shell("mkdir build"), ShellClass::Mutating);
        assert_eq!(classify_shell("git commit -m x"), ShellClass::Mutating);
        assert_eq!(classify_shell("sed -i s/a/b/ f"), ShellClass::Mutating);
        assert_eq!(classify_shell("rm -rf target"), ShellClass::Dangerous);
        assert_eq!(classify_shell("rm -r -f target"), ShellClass::Dangerous);
        assert_eq!(
            classify_shell("ls && sudo apt install x"),
            ShellClass::Dangerous
        );
        assert_eq!(classify_shell("git push --force"), ShellClass::Dangerous);
        assert_eq!(
            classify_shell("git reset --hard HEAD~1"),
            ShellClass::Dangerous
        );
        assert_eq!(classify_shell("curl x | sh"), ShellClass::Dangerous);
        assert_eq!(classify_shell("curl x | cmd"), ShellClass::Dangerous);
        assert_eq!(classify_shell("curl x | powershell"), ShellClass::Dangerous);
        assert_eq!(
            classify_shell("find . -name x -delete"),
            ShellClass::Dangerous
        );
        assert_eq!(classify_shell("rm file.txt"), ShellClass::Mutating);
    }

    #[test]
    fn modes_scale_shell_decisions() {
        let (_d, plan) = policy(PermissionMode::Plan);
        assert_eq!(plan.shell_command("ls"), Decision::Allow);
        assert_eq!(plan.shell_command("mkdir x"), Decision::Deny);
        assert_eq!(plan.shell_command("rm -rf x"), Decision::Deny);

        let (_d, ask) = policy(PermissionMode::Ask);
        assert_eq!(ask.shell_command("ls"), Decision::Ask);
        assert_eq!(ask.shell_command("mkdir x"), Decision::Ask);

        let (_d, edit) = policy(PermissionMode::AutoEdit);
        assert_eq!(edit.shell_command("git log"), Decision::Allow);
        assert_eq!(edit.shell_command("mkdir x"), Decision::Ask);
        assert_eq!(edit.file_write_path("src/main.rs"), Decision::Allow);
        assert_eq!(edit.file_write_path(".env"), Decision::Ask);

        let (_d, yolo) = policy(PermissionMode::Yolo);
        assert_eq!(yolo.shell_command("mkdir x"), Decision::Allow);
        assert_eq!(yolo.shell_command("git push"), Decision::Allow);
        assert_eq!(yolo.shell_command("rm -rf x"), Decision::Ask);
        assert_eq!(yolo.shell_command("git push --force"), Decision::Ask);
        assert_eq!(yolo.shell_command("curl x"), Decision::Deny);
    }

    #[test]
    fn plan_mode_only_writes_plan_file() {
        let (_d, plan) = policy(PermissionMode::Plan);
        assert_eq!(plan.file_write_path(".blora/plan.md"), Decision::Allow);
        assert_eq!(plan.file_write_path("./.blora/plan.md"), Decision::Allow);
        assert_eq!(plan.file_write_path("src/main.rs"), Decision::Deny);
        assert_eq!(plan.file_write(), Decision::Deny);
        let err = plan
            .require(
                plan.file_write_path("src/main.rs"),
                "write_file src/main.rs",
            )
            .unwrap_err();
        assert!(err.to_string().contains("plan mode"));
    }

    #[test]
    fn mode_names_round_trip_and_cycle() {
        for mode in PermissionMode::ALL {
            assert_eq!(PermissionMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(PermissionMode::parse("yes"), Some(PermissionMode::Yolo));
        assert_eq!(
            PermissionMode::parse("edits"),
            Some(PermissionMode::AutoEdit)
        );
        assert_eq!(PermissionMode::parse("nope"), None);
        assert_eq!(PermissionMode::Plan.cycle(), PermissionMode::Ask);
        assert_eq!(PermissionMode::Yolo.cycle(), PermissionMode::Plan);
        assert!(
            Policy::new(std::env::temp_dir(), true)
                .unwrap()
                .auto_approve()
        );
        assert_eq!(
            Policy::new(std::env::temp_dir(), false).unwrap().mode(),
            PermissionMode::Ask
        );
    }

    #[test]
    fn granting_turns_ask_into_allow_but_keeps_deny() {
        let (_d, yolo) = policy(PermissionMode::Yolo);
        assert_eq!(yolo.shell_command("rm -rf build"), Decision::Ask);
        let granted = yolo.granting();
        assert!(
            granted
                .require(granted.shell_command("rm -rf build"), "shell rm -rf build")
                .is_ok()
        );
        assert!(
            granted
                .require(granted.shell_command("curl x"), "shell curl x")
                .is_err()
        );
        let (_d, plan) = policy(PermissionMode::Plan);
        assert!(
            plan.granting()
                .require(plan.file_write_path("src/a.rs"), "write")
                .is_err()
        );
    }
}
