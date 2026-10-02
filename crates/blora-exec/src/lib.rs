// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Local process and filesystem backend. All paths are resolved through policy.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use blora_policy::Policy;
use blora_types::{BloraError, Result};
mod glob;
mod host;
mod isolation;
mod media;
mod process;
mod worktree;

use regex::Regex;
use walkdir::WalkDir;

pub use glob::glob_match;
pub use host::{
    HostShell, command, host_shell, open_url, path_is_absolute, quote, shell_command, terminate_pid,
};
pub use isolation::{Isolation, remote_command, remote_host, remote_root};
pub use process::ProcessInfo;
pub use worktree::WorktreeHandle;

const MAX_FILE_BYTES: usize = 1_000_000;
const MAX_SEARCH_MATCHES: usize = 80;
const MAX_GLOB_MATCHES: usize = 200;
const MAX_SHELL_BYTES: usize = 200_000;
const DEFAULT_SHELL_TIMEOUT: Duration = Duration::from_secs(30);

/// How [`LocalBackend::shell_prepared`] waits for the command.
#[derive(Clone, Copy, Debug)]
pub enum ShellKind {
    Foreground(Option<Duration>),
    Background,
    Pty,
}

/// Captured shell output plus whether the process exited zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellOutput {
    pub text: String,
    pub success: bool,
}

#[derive(Clone, Debug)]
pub struct LocalBackend {
    policy: Policy,
    isolation: Isolation,
}

impl LocalBackend {
    #[must_use]
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            isolation: Isolation::Local,
        }
    }

    #[must_use]
    pub fn with_isolation(mut self, isolation: Isolation) -> Self {
        self.isolation = isolation;
        self
    }

    #[must_use]
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    #[must_use]
    pub fn isolation(&self) -> Isolation {
        self.isolation
    }

    pub fn read_file(&self, path: &str) -> Result<String> {
        self.policy.require(
            self.policy.file_read_path(path),
            &format!("read_file {path}"),
        )?;
        let path = self.policy.resolve(path)?;
        if self.isolation == Isolation::Remote {
            return self.remote_capture(&format!("cat {}", path.display()));
        }
        let bytes = std::fs::read(&path).map_err(BloraError::exec)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(BloraError::Exec(format!(
                "file exceeds {MAX_FILE_BYTES} bytes: {}",
                path.display()
            )));
        }
        let label = relative_slashes(&path, self.policy.workspace());
        match String::from_utf8(bytes) {
            Ok(text) => Ok(text),
            Err(err) => Ok(media::describe_bytes(&label, &err.into_bytes())),
        }
    }

    pub fn write_file(&self, path: &str, contents: &str) -> Result<()> {
        self.policy.require(
            self.policy.file_write_path(path),
            &format!("write_file {path}"),
        )?;
        let path = self.resolve_for_write(path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(BloraError::exec)?;
        }
        if self.isolation == Isolation::Remote {
            return self.remote_write(&path, contents);
        }
        std::fs::write(&path, contents).map_err(BloraError::exec)
    }

    pub fn list_dir(&self, path: &str) -> Result<String> {
        self.policy.require(self.policy.file_read(), "list_dir")?;
        let path = self.policy.resolve(path)?;
        if self.isolation == Isolation::Remote {
            return self.remote_capture(&format!("ls -1a {}", path.display()));
        }
        let mut names = Vec::new();
        let entries = std::fs::read_dir(&path).map_err(BloraError::exec)?;
        for entry in entries {
            let entry = entry.map_err(BloraError::exec)?;
            let file_type = entry.file_type().map_err(BloraError::exec)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if file_type.is_dir() {
                names.push(format!("{name}/"));
            } else {
                names.push(name);
            }
        }
        names.sort();
        Ok(names.join("\n"))
    }

    pub fn search(&self, pattern: &str, path: Option<&str>) -> Result<String> {
        self.search_with(pattern, path, None, MAX_SEARCH_MATCHES)
    }

    /// Regex search over workspace files. `include` is a glob (see
    /// [`glob_match`]) that narrows which files are scanned; `max` caps the
    /// number of matching lines returned.
    pub fn search_with(
        &self,
        pattern: &str,
        path: Option<&str>,
        include: Option<&str>,
        max: usize,
    ) -> Result<String> {
        self.policy.require(self.policy.search(), "search")?;
        let root = self.policy.resolve(path.unwrap_or("."))?;
        let regex = Regex::new(pattern).map_err(BloraError::exec)?;
        let include = include.map(str::trim).filter(|s| !s.is_empty());
        let max = max.clamp(1, 1000);
        let mut matches = Vec::new();
        let mut files_scanned = 0usize;
        for entry in WalkDir::new(&root)
            .sort_by_file_name()
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if should_skip(entry.path()) {
                continue;
            }
            let rel_to_root = relative_slashes(entry.path(), &root);
            if include.is_some_and(|glob| !glob_match(glob, &rel_to_root)) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            files_scanned += 1;
            for (idx, line) in text.lines().enumerate() {
                if regex.is_match(line) {
                    let rel = entry
                        .path()
                        .strip_prefix(self.policy.workspace())
                        .unwrap_or(entry.path());
                    let shown: String = line.chars().take(400).collect();
                    matches.push(format!("{}:{}:{shown}", rel.display(), idx + 1));
                    if matches.len() >= max {
                        matches.push(format!(
                            "… truncated at {max} matches; narrow with include (glob) or a tighter pattern …"
                        ));
                        return Ok(matches.join("\n"));
                    }
                }
            }
        }
        if matches.is_empty() {
            Ok(format!("no matches ({files_scanned} files scanned)"))
        } else {
            Ok(matches.join("\n"))
        }
    }

    /// Files under `path` whose relative path matches `pattern`, newest first.
    pub fn glob(&self, pattern: &str, path: Option<&str>) -> Result<String> {
        self.policy.require(self.policy.search(), "glob")?;
        let root = self.policy.resolve(path.unwrap_or("."))?;
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return Err(BloraError::Exec(
                "glob pattern must not be empty".to_owned(),
            ));
        }
        let mut found: Vec<(std::time::SystemTime, String)> = Vec::new();
        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            if !entry.file_type().is_file() || should_skip(entry.path()) {
                continue;
            }
            let rel_to_root = relative_slashes(entry.path(), &root);
            if !glob_match(pattern, &rel_to_root) {
                continue;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::UNIX_EPOCH);
            found.push((
                modified,
                relative_slashes(entry.path(), self.policy.workspace()),
            ));
        }
        if found.is_empty() {
            return Ok("no matches".to_owned());
        }
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let total = found.len();
        let mut lines: Vec<String> = found
            .into_iter()
            .take(MAX_GLOB_MATCHES)
            .map(|(_, path)| path)
            .collect();
        if total > MAX_GLOB_MATCHES {
            lines.push(format!(
                "… {} more; narrow the pattern or path …",
                total - MAX_GLOB_MATCHES
            ));
        }
        Ok(lines.join("\n"))
    }

    pub fn shell(&self, command: &str) -> Result<String> {
        self.shell_with_timeout(command, None)
    }

    pub fn shell_with_timeout(&self, command: &str, timeout: Option<Duration>) -> Result<String> {
        Ok(self
            .shell_prepared(command, ".", ShellKind::Foreground(timeout))?
            .text)
    }

    /// Run `command` with the workspace-relative directory `cwd` as its start
    /// directory. A bare `cd` is rewritten so `~` stays at the workspace root
    /// and the destination must already be a directory inside the workspace.
    /// Policy is applied to the caller's command, not to the directory prefix.
    pub fn shell_prepared(&self, command: &str, cwd: &str, kind: ShellKind) -> Result<ShellOutput> {
        self.policy.require(
            self.policy.shell_command(command),
            &format!("shell {command}"),
        )?;
        if command.trim().is_empty() {
            return Err(BloraError::Exec("empty command".to_owned()));
        }
        let effective = self.normalize_cwd(cwd)?;
        let (run, at) = if let Some(target) = plain_cd_target(command) {
            let raw = match target {
                CdTarget::Root => ".".to_owned(),
                CdTarget::Path(path) => join_cwd(&effective, &path),
            };
            let dest = self.normalize_cwd(&raw)?;
            (host::cd_into(&dest), ".".to_owned())
        } else {
            (command.to_owned(), effective)
        };
        let run = wrap_cwd(&at, &run);
        match kind {
            ShellKind::Foreground(timeout) => {
                let (text, success) = self.spawn_shell(&run, timeout)?;
                Ok(ShellOutput { text, success })
            }
            ShellKind::Background => Ok(ShellOutput {
                text: self.spawn_background_unchecked(&run)?,
                success: true,
            }),
            ShellKind::Pty => {
                let (text, success) = self
                    .clone()
                    .with_isolation(Isolation::Pty)
                    .spawn_shell(&run, None)?;
                Ok(ShellOutput { text, success })
            }
        }
    }

    /// Workspace-relative directory, or `.` for the workspace root.
    /// The path must exist, be a directory, and stay inside the workspace.
    pub fn normalize_cwd(&self, cwd: &str) -> Result<String> {
        let path = self.policy.resolve(cwd)?;
        if !path.is_dir() {
            return Err(BloraError::Exec(format!("cwd is not a directory: {cwd}")));
        }
        let rel = path
            .strip_prefix(self.policy.workspace())
            .map_err(|_| BloraError::Policy(format!("path escapes workspace: {cwd}")))?;
        let text = rel
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if text.is_empty() {
            Ok(".".to_owned())
        } else {
            Ok(text)
        }
    }

    fn spawn_shell(&self, command: &str, timeout: Option<Duration>) -> Result<(String, bool)> {
        let timeout = timeout.unwrap_or(DEFAULT_SHELL_TIMEOUT);
        if command.trim().is_empty() {
            return Err(BloraError::Exec("empty command".to_owned()));
        }
        let mut child = self
            .isolation
            .shell_command(self.policy.workspace(), command)?
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(BloraError::exec)?;
        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_handle = thread::spawn(move || read_capped(stdout, MAX_SHELL_BYTES));
        let stderr_handle = thread::spawn(move || read_capped(stderr, MAX_SHELL_BYTES));

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let status = child.wait();
            let _ = tx.send(status);
        });
        let status = match rx.recv_timeout(timeout) {
            Ok(status) => status.map_err(BloraError::exec)?,
            Err(_) => {
                let _ = terminate_pid(pid);
                return Err(BloraError::Exec(format!(
                    "command timed out after {}s",
                    timeout.as_secs()
                )));
            }
        };
        let stdout = stdout_handle
            .join()
            .unwrap_or_else(|_| Ok(String::new()))
            .unwrap_or_default();
        let stderr = stderr_handle
            .join()
            .unwrap_or_else(|_| Ok(String::new()))
            .unwrap_or_default();
        let mut out = String::new();
        if !stdout.is_empty() {
            out.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("stderr:\n");
            out.push_str(&stderr);
        }
        let success = status.success();
        if !success {
            out.push_str(&format!("\nexit {}", status.code().unwrap_or(-1)));
        }
        if out.is_empty() {
            out = "(no output)".to_owned();
        }
        Ok((out, success))
    }

    pub fn git_status(&self) -> Result<String> {
        self.policy.require(self.policy.file_read(), "git_status")?;
        self.git(&["status", "--short", "--branch"])
    }

    pub fn git_branch_counts(&self) -> Result<String> {
        self.policy
            .require(self.policy.file_read(), "git_branch_counts")?;
        self.git(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=no",
        ])
    }

    pub fn git_diff(&self) -> Result<String> {
        self.policy.require(self.policy.file_read(), "git_diff")?;
        self.git(&["diff", "--stat", "HEAD"])
    }

    pub fn git_commit_context(&self) -> Result<String> {
        self.policy
            .require(self.policy.file_read(), "git_commit_context")?;
        let diff = self.git_mutate(&["diff", "--no-ext-diff", "HEAD", "--"])?;
        let untracked = self.git_mutate(&["ls-files", "--others", "--exclude-standard"])?;
        let mut context = String::new();
        context.push_str("Tracked changes:\n");
        context.push_str(&diff.chars().take(16000).collect::<String>());
        context.push_str("\nUntracked file names:\n");
        context.push_str(&untracked.chars().take(2000).collect::<String>());
        if diff.trim().is_empty() && untracked.trim().is_empty() {
            return Err(BloraError::Exec("没有可用于生成提交信息的变更".to_owned()));
        }
        Ok(context)
    }

    pub fn git_numstat(&self) -> Result<String> {
        self.policy
            .require(self.policy.file_read(), "git_numstat")?;
        self.git(&["diff", "--numstat", "HEAD"])
    }

    pub fn git_log(&self) -> Result<String> {
        self.policy.require(self.policy.file_read(), "git_log")?;
        self.git(&[
            "log",
            "--graph",
            "--all",
            "--topo-order",
            "--decorate=short",
            "--color=never",
            "-100",
            "--oneline",
        ])
    }

    pub fn git_branch(&self) -> Result<String> {
        self.policy.require(self.policy.file_read(), "git_branch")?;
        self.git(&["branch", "-vv"])
    }

    pub fn git_stashes(&self) -> Result<String> {
        self.policy
            .require(self.policy.file_read(), "git_stashes")?;
        self.git(&["stash", "list", "--format=%gd"])
    }

    /// Header-sized Git snapshot. Three commands, no diff, log, or directory listing.
    /// Fails when `path` is not a Git work tree so callers can hide the indicator.
    pub fn git_indicator(&self) -> Result<GitIndicator> {
        self.policy
            .require(self.policy.file_read(), "git_indicator")?;
        let status = self.git_strict(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=normal",
        ])?;
        // A repository with no commits has no HEAD. Line counts are then zero.
        let numstat = self
            .git_strict(&["diff", "--numstat", "HEAD"])
            .unwrap_or_default();
        let stashes = self
            .git_strict(&["stash", "list", "--format=%gd"])
            .unwrap_or_default();
        Ok(parse_git_indicator(&status, &numstat, &stashes))
    }

    /// Like [`Self::git`], but a non-zero exit is an error and empty output stays empty.
    fn git_strict(&self, args: &[&str]) -> Result<String> {
        let output = command("git")
            .args(args)
            .current_dir(self.policy.workspace())
            .env("LC_ALL", "C")
            .output()
            .map_err(BloraError::exec)?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            let text = err.trim();
            return Err(BloraError::Exec(if text.is_empty() {
                "git failed".to_owned()
            } else {
                text.to_owned()
            }));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn git_sync(&self, pull: bool) -> Result<String> {
        self.policy.require(self.policy.file_write(), "git_sync")?;
        let upstream = self.git_mutate(&[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ])?;
        let upstream = upstream.trim();
        let (remote, branch) = upstream
            .split_once('/')
            .ok_or_else(|| BloraError::Exec("无法识别当前分支的上游分支".to_owned()))?;
        if pull {
            let status = self.git_mutate(&["status", "--porcelain"])?;
            if !status.trim().is_empty() {
                return Err(BloraError::Exec(
                    "工作区存在未提交更改，请先提交或暂存后拉取".to_owned(),
                ));
            }
            self.git_mutate(&["pull", "--ff-only", "--", remote, branch])
        } else {
            self.git_mutate(&["push", "--", remote, &format!("HEAD:{branch}")])
        }
    }

    pub fn git_commit_selected(&self, message: &str, selected: &[String]) -> Result<String> {
        self.policy
            .require(self.policy.file_write(), "git_commit_selected")?;
        if message.trim().is_empty() {
            return Err(BloraError::Exec("commit message is empty".to_owned()));
        }
        if selected.is_empty() {
            return Err(BloraError::Exec("没有勾选要提交的文件".to_owned()));
        }
        let mut args = vec!["add", "--"];
        args.extend(selected.iter().map(String::as_str));
        self.git_mutate(&args)?;
        let mut args = vec!["commit", "-m", message.trim(), "--only", "--"];
        args.extend(selected.iter().map(String::as_str));
        self.git_mutate(&args)
    }

    pub fn git_worktree(&self, action: &str, path: Option<&str>) -> Result<String> {
        match action {
            "list" | "" => {
                self.policy
                    .require(self.policy.file_read(), "git_worktree")?;
                self.git(&["worktree", "list"])
            }
            "add" => {
                let path =
                    path.ok_or_else(|| BloraError::Exec("worktree add needs path".to_owned()))?;
                self.policy.require(
                    self.policy.file_write(),
                    &format!("git worktree add {path}"),
                )?;
                let resolved = self.resolve_for_write(path)?;
                self.git(&[
                    "worktree",
                    "add",
                    "--detach",
                    &resolved.display().to_string(),
                    "HEAD",
                ])
            }
            other => Err(BloraError::Exec(format!(
                "unknown git_worktree action: {other}"
            ))),
        }
    }

    /// Replace `old` with `new` in `path`. Without `replace_all` the match must be
    /// unique so an ambiguous edit cannot land in the wrong place. Returns the
    /// number of replacements made.
    pub fn apply_patch(
        &self,
        path: &str,
        old: &str,
        new: &str,
        replace_all: bool,
    ) -> Result<usize> {
        if old.is_empty() {
            return Err(BloraError::Exec("old_string must not be empty".to_owned()));
        }
        if old == new {
            return Err(BloraError::Exec(
                "old_string and new_string are identical".to_owned(),
            ));
        }
        let contents = self.read_file(path)?;
        let count = contents.matches(old).count();
        if count == 0 {
            return Err(BloraError::Exec(format!(
                "patch target not found in {path}; re-read the file and copy the exact text"
            )));
        }
        if count > 1 && !replace_all {
            return Err(BloraError::Exec(format!(
                "old_string matches {count} places in {path}; add surrounding lines to make it unique or set replace_all"
            )));
        }
        let updated = if replace_all {
            contents.replace(old, new)
        } else {
            contents.replacen(old, new, 1)
        };
        self.write_file(path, &updated)?;
        Ok(if replace_all { count } else { 1 })
    }

    pub fn shell_pty(&self, command: &str) -> Result<String> {
        self.clone().with_isolation(Isolation::Pty).shell(command)
    }

    fn remote_capture(&self, command: &str) -> Result<String> {
        let output = isolation::remote_command(self.policy.workspace(), command)?
            .output()
            .map_err(BloraError::exec)?;
        let mut out = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.stderr.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        if !output.status.success() && out.is_empty() {
            return Err(BloraError::Exec(format!(
                "remote command failed: {command}"
            )));
        }
        Ok(out)
    }

    fn remote_write(&self, path: &Path, contents: &str) -> Result<()> {
        let mut child = isolation::remote_command(
            self.policy.workspace(),
            &format!("cat > {}", path.display()),
        )?
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(BloraError::exec)?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin
                .write_all(contents.as_bytes())
                .map_err(BloraError::exec)?;
        }
        let status = child.wait().map_err(BloraError::exec)?;
        if status.success() {
            Ok(())
        } else {
            Err(BloraError::Exec(format!(
                "remote write failed: {}",
                path.display()
            )))
        }
    }

    fn git_mutate(&self, args: &[&str]) -> Result<String> {
        let output = command("git")
            .args(args)
            .current_dir(self.policy.workspace())
            .output()
            .map_err(BloraError::exec)?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            return Err(BloraError::Exec(text));
        }
        Ok(text)
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let output = command("git")
            .args(args)
            .current_dir(self.policy.workspace())
            .output()
            .map_err(BloraError::exec)?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.stderr.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        if text.trim().is_empty() {
            text = "(clean)".to_owned();
        }
        Ok(text)
    }

    fn resolve_for_write(&self, path: &str) -> Result<PathBuf> {
        let requested = path.trim();
        let raw = PathBuf::from(requested);
        let candidate = if raw.is_absolute() {
            raw
        } else {
            self.policy.workspace().join(raw)
        };
        if let Some(parent) = candidate.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        self.policy.resolve(requested).or_else(|_| {
            let normalized = {
                use std::path::Component;
                let mut out = PathBuf::new();
                for component in candidate.components() {
                    match component {
                        Component::ParentDir => {
                            out.pop();
                        }
                        Component::CurDir => {}
                        other => out.push(other.as_os_str()),
                    }
                }
                out
            };
            if !normalized.starts_with(self.policy.workspace()) {
                return Err(BloraError::Policy(format!(
                    "path escapes workspace: {requested}"
                )));
            }
            Ok(normalized)
        })
    }
}

/// Where a bare `cd` goes. `Root` is the workspace, including `cd` and `cd ~`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CdTarget {
    Root,
    Path(String),
}

/// Directory a later shell should start in.
///
/// `effective` is the directory this command already started in. A successful
/// bare `cd` moves from there; every other command stays put. `~` is the
/// workspace root, not the account home.
#[must_use]
pub fn shell_cwd_after(effective: &str, command: &str, success: bool) -> String {
    if success && let Some(target) = plain_cd_target(command) {
        return match target {
            CdTarget::Root => ".".to_owned(),
            CdTarget::Path(path) => join_cwd(effective, &path),
        };
    }
    effective.to_owned()
}

/// Join `target` onto `current`. Absolute targets are returned unchanged so
/// the caller can reject anything outside the workspace.
#[must_use]
pub fn join_cwd(current: &str, target: &str) -> String {
    let target = target.trim();
    if let Some(rest) = target
        .strip_prefix("~/")
        .or_else(|| target.strip_prefix("~\\"))
    {
        return rest.trim_start_matches(['/', '\\']).to_owned();
    }
    if path_is_absolute(target) {
        return target.to_owned();
    }
    let base = current.trim().trim_end_matches('/');
    if base.is_empty() || base == "." {
        target.to_owned()
    } else {
        format!("{base}/{target}")
    }
}

/// `Some` when `command` is only `cd` plus an optional directory.
/// Compound commands (`cd x && ls`) return `None` so they are not rewritten.
#[must_use]
pub fn plain_cd_target(command: &str) -> Option<CdTarget> {
    let trimmed = command.trim();
    let rest = trimmed.strip_prefix("cd")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    if rest.chars().any(|ch| {
        matches!(
            ch,
            '&' | '|' | ';' | '\n' | '`' | '$' | '>' | '<' | '(' | ')'
        )
    }) {
        return None;
    }
    let quoted = rest.len() >= 2
        && ((rest.starts_with('"') && rest.ends_with('"'))
            || (rest.starts_with('\'') && rest.ends_with('\'')));
    let target = if quoted {
        &rest[1..rest.len() - 1]
    } else {
        rest
    };
    if target.is_empty() || target == "~" || target == "~/" {
        return Some(CdTarget::Root);
    }
    if !quoted && target.split_whitespace().nth(1).is_some() {
        return None;
    }
    if let Some(rest) = target
        .strip_prefix("~/")
        .or_else(|| target.strip_prefix("~\\"))
    {
        return Some(CdTarget::Path(
            rest.trim_start_matches(['/', '\\']).to_owned(),
        ));
    }
    Some(CdTarget::Path(target.to_owned()))
}

fn wrap_cwd(cwd: &str, command: &str) -> String {
    host::chain(cwd, command)
}

fn should_skip(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("target" | ".git" | "node_modules" | ".blora")
        )
    })
}

/// `path` relative to `base` with forward slashes, for glob matching and output.
fn relative_slashes(path: &Path, base: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn read_capped<R: Read>(reader: Option<R>, max: usize) -> Result<String> {
    let Some(mut reader) = reader else {
        return Ok(String::new());
    };
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).map_err(BloraError::exec)?;
    if buf.len() > max {
        buf.truncate(max);
        buf.extend_from_slice(b"\n... truncated ...");
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Counts the header indicator needs. Parsed from porcelain v2, which stays
/// machine-readable when Git's human status is localized.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitIndicator {
    pub branch: String,
    pub added: usize,
    pub removed: usize,
    pub ahead: usize,
    pub behind: usize,
    pub stashes: usize,
    pub clean: bool,
}

/// `status_v2` is `git status --porcelain=v2 --branch`. `numstat` is
/// `git diff --numstat HEAD`. `stashes` is `git stash list --format=%gd`.
#[must_use]
pub fn parse_git_indicator(status_v2: &str, numstat: &str, stashes: &str) -> GitIndicator {
    let mut branch = String::new();
    let mut ahead = 0usize;
    let mut behind = 0usize;
    let mut clean = true;
    for line in status_v2.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("# branch.head ") {
            branch = name.trim().to_owned();
            continue;
        }
        if let Some(counts) = line.strip_prefix("# branch.ab ") {
            let mut parts = counts.split_whitespace();
            if let (Some(add), Some(sub)) = (parts.next(), parts.next()) {
                ahead = add
                    .strip_prefix('+')
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                behind = sub
                    .strip_prefix('-')
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
            }
            continue;
        }
        if !line.starts_with('#') {
            clean = false;
        }
    }
    let (added, removed) = numstat_totals(numstat);
    let stash_count = stashes
        .lines()
        .filter(|line| line.starts_with("stash@{"))
        .count();
    GitIndicator {
        branch,
        added,
        removed,
        ahead,
        behind,
        stashes: stash_count,
        clean,
    }
}

fn numstat_totals(numstat: &str) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    for line in numstat.lines() {
        let mut columns = line.split('\t');
        let (Some(additions), Some(deletions), Some(_path)) =
            (columns.next(), columns.next(), columns.next())
        else {
            continue;
        };
        added += additions.parse::<usize>().unwrap_or(0);
        removed += deletions.parse::<usize>().unwrap_or(0);
    }
    (added, removed)
}

/// Local filesystem/process backend. Future sandbox/container backends
/// should implement the same method set.
pub trait ExecutionBackend {
    fn read_file(&self, path: &str) -> Result<String>;
    fn write_file(&self, path: &str, contents: &str) -> Result<()>;
    fn list_dir(&self, path: &str) -> Result<String>;
    fn search(&self, pattern: &str, path: Option<&str>) -> Result<String>;
    fn shell(&self, command: &str) -> Result<String>;
}

impl ExecutionBackend for LocalBackend {
    fn read_file(&self, path: &str) -> Result<String> {
        LocalBackend::read_file(self, path)
    }

    fn write_file(&self, path: &str, contents: &str) -> Result<()> {
        LocalBackend::write_file(self, path, contents)
    }

    fn list_dir(&self, path: &str) -> Result<String> {
        LocalBackend::list_dir(self, path)
    }

    fn search(&self, pattern: &str, path: Option<&str>) -> Result<String> {
        LocalBackend::search(self, pattern, path)
    }

    fn shell(&self, command: &str) -> Result<String> {
        LocalBackend::shell(self, command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_policy::Policy;
    use std::process::Command;

    #[test]
    fn reads_and_writes_inside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::new(dir.path(), true).unwrap();
        let backend = LocalBackend::new(policy);
        backend.write_file("notes.txt", "hello").unwrap();
        assert_eq!(backend.read_file("notes.txt").unwrap(), "hello");
        assert!(backend.list_dir(".").unwrap().contains("notes.txt"));
        assert!(backend.search("hello", None).unwrap().contains("notes.txt"));
        backend
            .apply_patch("notes.txt", "hello", "hallo", false)
            .unwrap();
        assert_eq!(backend.read_file("notes.txt").unwrap(), "hallo");
    }

    #[test]
    fn glob_and_search_include_filter_files() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::new(dir.path(), true).unwrap();
        let backend = LocalBackend::new(policy);
        backend.write_file("src/lib.rs", "fn hit() {}\n").unwrap();
        backend
            .write_file("src/deep/more.rs", "fn hit() {}\n")
            .unwrap();
        backend.write_file("notes/hit.md", "hit\n").unwrap();
        backend
            .write_file("target/skip.rs", "fn hit() {}\n")
            .unwrap();

        let rs = backend.glob("**/*.rs", None).unwrap();
        assert!(rs.contains("src/lib.rs"));
        assert!(rs.contains("src/deep/more.rs"));
        assert!(!rs.contains("notes/hit.md"));
        assert!(!rs.contains("target/skip.rs"), "target/ is skipped");
        let top = backend.glob("src/*.rs", None).unwrap();
        assert!(top.contains("src/lib.rs"));
        assert!(!top.contains("more.rs"));
        assert_eq!(backend.glob("*.py", None).unwrap(), "no matches");

        let only_md = backend.search_with("hit", None, Some("*.md"), 10).unwrap();
        assert!(only_md.contains("notes/hit.md"));
        assert!(!only_md.contains("lib.rs"));
        let capped = backend.search_with("hit", None, None, 1).unwrap();
        assert!(capped.contains("truncated at 1 matches"));
    }

    #[test]
    fn apply_patch_requires_unique_match() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::new(dir.path(), true).unwrap();
        let backend = LocalBackend::new(policy);
        backend.write_file("a.txt", "x\nx\n").unwrap();
        let err = backend.apply_patch("a.txt", "x", "y", false).unwrap_err();
        assert!(err.to_string().contains("matches 2 places"));
        assert_eq!(backend.apply_patch("a.txt", "x", "y", true).unwrap(), 2);
        assert_eq!(backend.read_file("a.txt").unwrap(), "y\ny\n");
        assert!(backend.apply_patch("a.txt", "zzz", "y", false).is_err());
    }

    #[test]
    fn shell_starts_in_the_requested_directory_and_rejects_escape() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalBackend::new(Policy::new(dir.path(), true).unwrap());
        backend.write_file("sub/note.txt", "inside\n").unwrap();
        let output = backend
            .shell_prepared("cat note.txt", "sub", ShellKind::Foreground(None))
            .unwrap();
        assert!(output.success);
        assert!(output.text.contains("inside"));
        assert!(backend.normalize_cwd("../..").is_err());
        assert!(backend.normalize_cwd("sub/note.txt").is_err());
    }

    #[test]
    fn bare_cd_stays_inside_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalBackend::new(Policy::new(dir.path(), true).unwrap());
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let moved = backend
            .shell_prepared("cd sub", ".", ShellKind::Foreground(None))
            .unwrap();
        assert!(moved.success);
        assert_eq!(shell_cwd_after(".", "cd sub", true), "sub");
        assert_eq!(shell_cwd_after("sub", "cd ~", true), ".");
        assert_eq!(shell_cwd_after("sub", "pwd", true), "sub");
        assert_eq!(shell_cwd_after("sub", "cd ..", false), "sub");
        assert!(plain_cd_target("cd sub && ls").is_none());
        assert_eq!(join_cwd("src", r"C:\Windows"), r"C:\Windows");
        assert_eq!(join_cwd("src", r"\\server\share"), r"\\server\share");
        assert_eq!(join_cwd("src", "lib.rs"), "src/lib.rs");
        let escaped = backend.shell_prepared("cd ..", ".", ShellKind::Foreground(None));
        assert!(escaped.is_err());
    }

    #[test]
    fn read_file_describes_png_instead_of_lossy_text() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalBackend::new(Policy::new(dir.path(), true).unwrap());
        let mut bytes = vec![0u8; 24];
        bytes[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes[12..16].copy_from_slice(b"IHDR");
        bytes[16..20].copy_from_slice(&8u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&4u32.to_be_bytes());
        let path = dir.path().join("icon.png");
        std::fs::write(&path, &bytes).unwrap();
        let text = backend.read_file("icon.png").unwrap();
        assert!(text.contains("image/png 8x4"));
        assert!(!text.contains('\u{fffd}'));
    }

    #[test]
    fn parses_porcelain_branch_ab_without_localized_status_text() {
        let status = "\
# branch.oid abc
# branch.head main
# branch.upstream origin/main
# branch.ab +154 -0
1 .M N... 100644 100644 100644 a b src/lib.rs
";
        let numstat = "12\t3\tsrc/lib.rs\n-\t-\timage.png\n4\t0\tnew file.rs\n";
        let info = parse_git_indicator(status, numstat, "stash@{0}\nstash@{1}\n");
        assert_eq!(info.branch, "main");
        assert_eq!((info.ahead, info.behind), (154, 0));
        assert_eq!((info.added, info.removed), (16, 3));
        assert_eq!(info.stashes, 2);
        assert!(!info.clean);

        let clean = parse_git_indicator("# branch.head main\n# branch.ab +5 -2\n", "", "");
        assert!(clean.clean);
        assert_eq!((clean.ahead, clean.behind), (5, 2));
    }

    #[test]
    fn git_indicator_reports_a_dirty_worktree_and_rejects_a_plain_directory() {
        let dir = tempfile::tempdir().unwrap();
        let plain = LocalBackend::new(Policy::new(dir.path(), true).unwrap());
        assert!(plain.git_indicator().is_err());

        assert!(
            Command::new("git")
                .args(["init"])
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(dir.path().join("notes.txt"), "hello\n").unwrap();
        let backend = LocalBackend::new(Policy::new(dir.path(), true).unwrap());
        let info = backend.git_indicator().unwrap();
        assert!(!info.clean);
        assert!(!info.branch.is_empty());

        if !chinese_locale_available() {
            return;
        }
        let output = Command::new("git")
            .args([
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=normal",
            ])
            .env("LC_ALL", "zh_CN.UTF-8")
            .env("LANG", "zh_CN.UTF-8")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.lines().any(|line| line.starts_with("# branch.head ")),
            "{text}"
        );
        let parsed = parse_git_indicator(&text, "", "");
        assert!(!parsed.clean);
        assert_eq!(parsed.branch, info.branch);
    }

    #[test]
    fn git_log_preserves_merge_topology_and_other_branch_refs() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-b", "main"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.invalid"]);
        git(&["commit", "--allow-empty", "-m", "root"]);
        git(&["checkout", "-b", "feature"]);
        git(&["commit", "--allow-empty", "-m", "feature commit"]);
        git(&["checkout", "main"]);
        git(&["commit", "--allow-empty", "-m", "main commit"]);
        git(&["merge", "--no-ff", "feature", "-m", "merge feature"]);
        git(&["checkout", "-b", "unmerged"]);
        git(&["commit", "--allow-empty", "-m", "unmerged commit"]);
        git(&["checkout", "main"]);
        let backend = LocalBackend::new(Policy::new(dir.path(), true).unwrap());
        let log = backend.git_log().unwrap();
        assert!(log.contains("|\\"), "{log}");
        assert!(log.contains("HEAD -> main"), "{log}");
        assert!(log.contains("unmerged"), "{log}");
        assert!(log.contains("feature commit"), "{log}");
        assert!(!log.contains('\x1b'));
    }

    fn chinese_locale_available() -> bool {
        Command::new("locale")
            .arg("-a")
            .output()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.to_ascii_lowercase().starts_with("zh_cn"))
            })
            .unwrap_or(false)
    }
}
