// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Local process and filesystem backend. All paths are resolved through policy.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use blora_policy::Policy;
use blora_types::{BloraError, Result};
mod glob;
mod isolation;
mod process;
mod worktree;

use regex::Regex;
use walkdir::WalkDir;

pub use glob::glob_match;
pub use isolation::{Isolation, remote_command, remote_host, remote_root};
pub use process::ProcessInfo;
pub use worktree::WorktreeHandle;

const MAX_FILE_BYTES: usize = 1_000_000;
const MAX_SEARCH_MATCHES: usize = 80;
const MAX_GLOB_MATCHES: usize = 200;
const MAX_SHELL_BYTES: usize = 200_000;
const DEFAULT_SHELL_TIMEOUT: Duration = Duration::from_secs(30);

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
        Ok(String::from_utf8_lossy(&bytes).into_owned())
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
            return Err(BloraError::Exec("glob pattern must not be empty".to_owned()));
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
        let timeout = timeout.unwrap_or(DEFAULT_SHELL_TIMEOUT);
        self.policy.require(
            self.policy.shell_command(command),
            &format!("shell {command}"),
        )?;
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
                let _ = Command::new("kill").arg(pid.to_string()).status();
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
        if !status.success() {
            out.push_str(&format!("\nexit {}", status.code().unwrap_or(-1)));
        }
        if out.is_empty() {
            out = "(no output)".to_owned();
        }
        Ok(out)
    }

    pub fn git_status(&self) -> Result<String> {
        self.policy.require(self.policy.file_read(), "git_status")?;
        self.git(&["status", "--short", "--branch"])
    }

    pub fn git_diff(&self) -> Result<String> {
        self.policy.require(self.policy.file_read(), "git_diff")?;
        self.git(&["diff", "--stat", "HEAD"])
    }

    pub fn git_numstat(&self) -> Result<String> {
        self.policy
            .require(self.policy.file_read(), "git_numstat")?;
        self.git(&["diff", "--numstat", "HEAD"])
    }

    pub fn git_log(&self) -> Result<String> {
        self.policy.require(self.policy.file_read(), "git_log")?;
        self.git(&["log", "-8", "--oneline"])
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

    pub fn git_stage(&self, path: &str, stage: bool) -> Result<()> {
        self.policy.require(self.policy.file_write(), "git_stage")?;
        let args = if stage {
            vec!["add", "--", path]
        } else {
            vec!["restore", "--staged", "--", path]
        };
        self.git_mutate(&args).map(|_| ())
    }

    pub fn git_commit(&self, message: &str) -> Result<String> {
        self.policy
            .require(self.policy.file_write(), "git_commit")?;
        if message.trim().is_empty() {
            return Err(BloraError::Exec("commit message is empty".to_owned()));
        }
        self.git_mutate(&["commit", "-m", message.trim()])
    }

    fn git_mutate(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
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

    fn git(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
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

    #[test]
    fn git_stage_targets_one_file_and_rejects_empty_commit_message() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(dir.path().join("one.txt"), "one").unwrap();
        std::fs::write(dir.path().join("two.txt"), "two").unwrap();
        let backend = LocalBackend::new(Policy::new(dir.path(), true).unwrap());
        backend.git_stage("one.txt", true).unwrap();
        let status = Command::new("git")
            .args(["status", "--short"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let status = String::from_utf8(status.stdout).unwrap();
        assert!(status.contains("A  one.txt"));
        assert!(status.contains("?? two.txt"));
        assert!(
            Command::new("git")
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "commit",
                    "-qm",
                    "init"
                ])
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(dir.path().join("one.txt"), "changed").unwrap();
        backend.git_stage("one.txt", true).unwrap();
        backend.git_stage("one.txt", false).unwrap();
        let status = Command::new("git")
            .args(["status", "--short"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            String::from_utf8(status.stdout)
                .unwrap()
                .contains(" M one.txt")
        );
        assert!(backend.git_commit("  ").is_err());
    }

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
        backend.write_file("src/deep/more.rs", "fn hit() {}\n").unwrap();
        backend.write_file("notes/hit.md", "hit\n").unwrap();
        backend.write_file("target/skip.rs", "fn hit() {}\n").unwrap();

        let rs = backend.glob("**/*.rs", None).unwrap();
        assert!(rs.contains("src/lib.rs"));
        assert!(rs.contains("src/deep/more.rs"));
        assert!(!rs.contains("notes/hit.md"));
        assert!(!rs.contains("target/skip.rs"), "target/ is skipped");
        let top = backend.glob("src/*.rs", None).unwrap();
        assert!(top.contains("src/lib.rs"));
        assert!(!top.contains("more.rs"));
        assert_eq!(backend.glob("*.py", None).unwrap(), "no matches");

        let only_md = backend
            .search_with("hit", None, Some("*.md"), 10)
            .unwrap();
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
}
