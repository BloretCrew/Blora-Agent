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
    /// OS-level sandbox via bubblewrap: read-only root, writable workspace, no network.
    Bwrap,
    /// Run the command in a disposable container with the workspace mounted.
    Container,
    /// Allocate a PTY via `script` so interactive programs see a terminal.
    Pty,
    /// Run commands on a remote host with `ssh` (`BLORA_REMOTE=user@host`).
    Remote,
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
            "bwrap" | "bubblewrap" => Self::Bwrap,
            "container" | "docker" => Self::Container,
            "pty" => Self::Pty,
            "remote" | "ssh" => Self::Remote,
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
            Self::Bwrap => bwrap_command(workspace, command),
            Self::Container => container_command(workspace, command),
            Self::Pty => pty_command(workspace, command),
            Self::Remote => remote_command(workspace, command),
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

/// Bubblewrap sandbox: the host filesystem is visible read-only, only the
/// workspace and a private /tmp are writable, and the network namespace is
/// unshared. Extra writable paths come from `BLORA_BWRAP_RW` (colon separated).
fn bwrap_command(workspace: &Path, command: &str) -> Result<Command> {
    let available = Command::new("bwrap")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !available {
        return Err(BloraError::Exec(
            "BLORA_EXEC=bwrap requires bubblewrap (`bwrap`) on PATH".to_owned(),
        ));
    }
    let workspace_str = workspace.display().to_string();
    let mut cmd = Command::new("bwrap");
    cmd.args(["--ro-bind", "/", "/"])
        .args(["--bind", &workspace_str, &workspace_str])
        .args(["--tmpfs", "/tmp"])
        .args(["--dev", "/dev"])
        .args(["--proc", "/proc"])
        .arg("--unshare-net")
        .arg("--unshare-pid")
        .arg("--die-with-parent")
        .arg("--new-session")
        .args(["--chdir", &workspace_str]);
    if let Ok(extra) = std::env::var("BLORA_BWRAP_RW") {
        for path in extra.split(':').filter(|p| !p.is_empty()) {
            cmd.args(["--bind", path, path]);
        }
    }
    cmd.args(["--setenv", "HOME", &workspace_str])
        .args(["--setenv", "LANG", "C"])
        .args(["--", "sh", "-c", command])
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
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

fn pty_command(workspace: &Path, command: &str) -> Result<Command> {
    let mut cmd = Command::new("script");
    cmd.args(["-qefc", command, "/dev/null"])
        .current_dir(workspace);
    Ok(cmd)
}

pub fn remote_host() -> Result<String> {
    std::env::var("BLORA_REMOTE")
        .map_err(|_| BloraError::Exec("set BLORA_REMOTE=user@host for remote execution".to_owned()))
}

pub fn remote_root(workspace: &Path) -> String {
    std::env::var("BLORA_REMOTE_ROOT").unwrap_or_else(|_| workspace.display().to_string())
}

pub fn remote_command(workspace: &Path, command: &str) -> Result<Command> {
    let host = remote_host()?;
    let root = remote_root(workspace);
    let script = format!("cd {root} && {command}");
    let mut cmd = Command::new("ssh");
    cmd.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=8",
        &host,
        "sh",
        "-lc",
        &script,
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

    #[test]
    fn pty_uses_script() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = Isolation::Pty.shell_command(dir.path(), "echo hi").unwrap();
        assert_eq!(cmd.get_program(), "script");
    }

    #[test]
    fn bwrap_builds_or_explains() {
        let dir = tempfile::tempdir().unwrap();
        match Isolation::Bwrap.shell_command(dir.path(), "true") {
            Ok(cmd) => {
                assert_eq!(cmd.get_program(), "bwrap");
                let args: Vec<String> = cmd
                    .get_args()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                assert!(args.iter().any(|a| a == "--unshare-net"));
                assert!(args.iter().any(|a| a == "--die-with-parent"));
            }
            Err(err) => assert!(err.to_string().contains("bubblewrap")),
        }
    }

    #[test]
    fn remote_requires_host() {
        let dir = tempfile::tempdir().unwrap();
        let err = Isolation::Remote
            .shell_command(dir.path(), "true")
            .unwrap_err();
        assert!(err.to_string().contains("BLORA_REMOTE"));
    }
}
