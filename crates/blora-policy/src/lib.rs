// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Fail-closed workspace policy. Destructive capabilities require approval
//! unless the operator opts into auto-approve.

use std::path::{Path, PathBuf};

use blora_types::{BloraError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

#[derive(Clone, Debug)]
pub struct Policy {
    workspace: PathBuf,
    auto_approve: bool,
}

impl Policy {
    pub fn new(workspace: impl Into<PathBuf>, auto_approve: bool) -> Result<Self> {
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
            auto_approve,
        })
    }

    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    #[must_use]
    pub fn auto_approve(&self) -> bool {
        self.auto_approve
    }

    #[must_use]
    pub fn granting(&self) -> Self {
        Self {
            workspace: self.workspace.clone(),
            auto_approve: true,
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
    pub fn file_write(&self) -> Decision {
        if self.auto_approve {
            Decision::Allow
        } else {
            Decision::Ask
        }
    }

    #[must_use]
    pub fn search(&self) -> Decision {
        Decision::Allow
    }

    #[must_use]
    pub fn shell(&self) -> Decision {
        if self.auto_approve {
            Decision::Allow
        } else {
            Decision::Ask
        }
    }

    pub fn require(&self, decision: Decision, summary: &str) -> Result<()> {
        match decision {
            Decision::Allow => Ok(()),
            Decision::Deny => Err(BloraError::Policy(summary.to_owned())),
            Decision::Ask => Err(BloraError::ApprovalRequired(summary.to_owned())),
        }
    }
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

    #[test]
    fn rejects_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::new(dir.path(), false).unwrap();
        assert!(policy.resolve("../etc/passwd").is_err());
        assert!(policy.resolve(".").is_ok());
    }
}
