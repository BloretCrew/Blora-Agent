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
mod process;
mod worktree;

use regex::Regex;
use walkdir::WalkDir;

pub use process::ProcessInfo;
pub use worktree::WorktreeHandle;

const MAX_FILE_BYTES: usize = 1_000_000;
const MAX_SEARCH_MATCHES: usize = 80;
const MAX_SHELL_BYTES: usize = 200_000;
const DEFAULT_SHELL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct LocalBackend {
    policy: Policy,
}

impl LocalBackend {
    #[must_use]
    pub fn new(policy: Policy) -> Self {
        Self { policy }
    }

    #[must_use]
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn read_file(&self, path: &str) -> Result<String> {
        self.policy.require(self.policy.file_read(), "read_file")?;
        let path = self.policy.resolve(path)?;
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
        self.policy
            .require(self.policy.file_write(), &format!("write_file {path}"))?;
        let path = self.resolve_for_write(path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(BloraError::exec)?;
        }
        std::fs::write(&path, contents).map_err(BloraError::exec)
    }

    pub fn list_dir(&self, path: &str) -> Result<String> {
        self.policy.require(self.policy.file_read(), "list_dir")?;
        let path = self.policy.resolve(path)?;
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
        self.policy.require(self.policy.search(), "search")?;
        let root = self.policy.resolve(path.unwrap_or("."))?;
        let regex = Regex::new(pattern).map_err(BloraError::exec)?;
        let mut matches = Vec::new();
        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if should_skip(entry.path()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            for (idx, line) in text.lines().enumerate() {
                if regex.is_match(line) {
                    let rel = entry
                        .path()
                        .strip_prefix(self.policy.workspace())
                        .unwrap_or(entry.path());
                    matches.push(format!("{}:{}:{line}", rel.display(), idx + 1));
                    if matches.len() >= MAX_SEARCH_MATCHES {
                        matches.push("… truncated …".to_owned());
                        return Ok(matches.join("\n"));
                    }
                }
            }
        }
        if matches.is_empty() {
            Ok("no matches".to_owned())
        } else {
            Ok(matches.join("\n"))
        }
    }

    pub fn shell(&self, command: &str) -> Result<String> {
        self.policy
            .require(self.policy.shell(), &format!("shell {command}"))?;
        if command.trim().is_empty() {
            return Err(BloraError::Exec("empty command".to_owned()));
        }
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(self.policy.workspace())
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
        let status = match rx.recv_timeout(DEFAULT_SHELL_TIMEOUT) {
            Ok(status) => status.map_err(BloraError::exec)?,
            Err(_) => {
                let _ = Command::new("kill").arg(pid.to_string()).status();
                return Err(BloraError::Exec(format!(
                    "command timed out after {}s",
                    DEFAULT_SHELL_TIMEOUT.as_secs()
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

    pub fn git_log(&self) -> Result<String> {
        self.policy.require(self.policy.file_read(), "git_log")?;
        self.git(&["log", "-8", "--oneline"])
    }

    pub fn git_branch(&self) -> Result<String> {
        self.policy.require(self.policy.file_read(), "git_branch")?;
        self.git(&["branch", "-vv"])
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

    pub fn apply_patch(&self, path: &str, old: &str, new: &str) -> Result<()> {
        let contents = self.read_file(path)?;
        if !contents.contains(old) {
            return Err(BloraError::Exec(format!(
                "patch target not found in {path}"
            )));
        }
        let updated = contents.replacen(old, new, 1);
        self.write_file(path, &updated)
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
    fn reads_and_writes_inside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::new(dir.path(), true).unwrap();
        let backend = LocalBackend::new(policy);
        backend.write_file("notes.txt", "hello").unwrap();
        assert_eq!(backend.read_file("notes.txt").unwrap(), "hello");
        assert!(backend.list_dir(".").unwrap().contains("notes.txt"));
        assert!(backend.search("hello", None).unwrap().contains("notes.txt"));
        backend.apply_patch("notes.txt", "hello", "hallo").unwrap();
        assert_eq!(backend.read_file("notes.txt").unwrap(), "hallo");
    }
}
