// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::{Path, PathBuf};

use blora_types::{BloraError, Result};

use crate::host::command;

/// Isolated Git worktree under `{workspace}/.blora/worktrees/{name}`.
#[derive(Clone, Debug)]
pub struct WorktreeHandle {
    pub path: PathBuf,
    source: PathBuf,
}

impl WorktreeHandle {
    pub fn create(source: impl AsRef<Path>, name: &str) -> Result<Self> {
        let source = source.as_ref();
        let safe: String = name
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
            .collect();
        if safe.is_empty() {
            return Err(BloraError::Exec("worktree name is empty".to_owned()));
        }
        let inside = command("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(source)
            .output()
            .map_err(BloraError::exec)?;
        if !inside.status.success() {
            return Err(BloraError::Exec(
                "workspace is not a git repository".to_owned(),
            ));
        }
        let path = source.join(".blora").join("worktrees").join(&safe);
        if path.exists() {
            return Ok(Self {
                path,
                source: source.to_path_buf(),
            });
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(BloraError::exec)?;
        }
        let output = command("git")
            .args([
                "worktree",
                "add",
                "--detach",
                &path.display().to_string(),
                "HEAD",
            ])
            .current_dir(source)
            .output()
            .map_err(BloraError::exec)?;
        if !output.status.success() {
            return Err(BloraError::Exec(format!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(Self {
            path,
            source: source.to_path_buf(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn remove(self) -> Result<()> {
        let output = command("git")
            .args([
                "worktree",
                "remove",
                "--force",
                &self.path.display().to_string(),
            ])
            .current_dir(&self.source)
            .output()
            .map_err(BloraError::exec)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(BloraError::Exec(format!(
                "git worktree remove failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_detached_worktree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README"), "hi").unwrap();
        let git = |args: &[&str]| {
            command("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success()
        };
        assert!(git(&["init"]));
        let _ = command("git")
            .args(["config", "user.email", "blora@example.test"])
            .current_dir(dir.path())
            .status();
        let _ = command("git")
            .args(["config", "user.name", "Blora"])
            .current_dir(dir.path())
            .status();
        assert!(git(&["add", "."]));
        assert!(git(&["commit", "-m", "init"]));
        let handle = WorktreeHandle::create(dir.path(), "ses_test").unwrap();
        assert!(handle.path().join("README").exists());
        handle.remove().unwrap();
    }
}
