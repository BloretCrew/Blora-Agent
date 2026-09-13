// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::Path;
use std::process::Command;

use blora_types::{BloraError, Result};

/// How shell commands are isolated from the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Isolation {
    #[default]
    Local,
    /// Drop extra env; use `unshare -n` when available so the command has no network.
    Sandbox,
    /// Run the command in a disposable container with the workspace mounted.
    Container,
}

impl Isolation {
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("BLORA_EXEC")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "sandbox" => Self::Sandbox,
            "container" | "docker" => Self::Container,
            _ => Self::Local,
        }
    }

    pub fn shell_command(self, workspace: &Path, command: &str) -> Result<Command> {
        match self {
            Self::Local => {
                let mut cmd = Command::new("sh");
                cmd.arg("-c").arg(command).current_dir(workspace);
                Ok(cmd)
            }
            Self::Sandbox => sandbox_command(workspace, command),
            Self::Container => container_command(workspace, command),
        }
    }
}

fn sandbox_command(workspace: &Path, command: &str) -> Result<Command> {
    let unshare_ok = Command::new("unshare")
        .args(["-n", "true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let mut cmd = if unshare_ok {
        let mut cmd = Command::new("unshare");
        cmd.args(["-n", "--", "sh", "-c", command]);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    };
    cmd.current_dir(workspace)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", workspace)
        .env("LANG", "C");
    Ok(cmd)
}

fn container_command(workspace: &Path, command: &str) -> Result<Command> {
    let image = std::env::var("BLORA_CONTAINER_IMAGE").unwrap_or_else(|_| "alpine:3.20".to_owned());
    let mount = format!("{}:/work", workspace.display());
    let docker = Command::new("docker")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if !docker.map(|status| status.success()).unwrap_or(false) {
        return Err(BloraError::Exec(
            "BLORA_EXEC=container requires a working docker CLI".to_owned(),
        ));
    }
    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "--rm",
        "--network=none",
        "-v",
        &mount,
        "-w",
        "/work",
        &image,
        "sh",
        "-c",
        command,
    ]);
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_env_names() {
        assert_eq!(Isolation::from_env(), Isolation::Local);
    }

    #[test]
    fn sandbox_builds_a_command() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = Isolation::Sandbox
            .shell_command(dir.path(), "true")
            .unwrap();
        assert!(cmd.get_program() == "unshare" || cmd.get_program() == "sh");
    }
}
