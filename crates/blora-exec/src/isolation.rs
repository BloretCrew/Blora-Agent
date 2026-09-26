// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::Path;
use std::process::Command;

use blora_types::{BloraError, Result};

use crate::host;

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
            Self::Local => Ok(host::shell_command(workspace, command)),
            Self::Sandbox => sandbox_command(workspace, command),
            Self::Bwrap => bwrap_command(workspace, command),
            Self::Container => container_command(workspace, command),
            Self::Pty => pty_command(workspace, command),
            Self::Remote => remote_command(workspace, command),
        }
    }
}

fn sandbox_command(workspace: &Path, command: &str) -> Result<Command> {
    let unshare_ok = host::command("unshare")
        .args(["-n", "true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let mut cmd = if unshare_ok {
        let mut cmd = host::command("unshare");
        cmd.args(["-n", "--", "sh", "-c", command]);
        cmd
    } else {
        host::shell_command(workspace, command)
    };
    let workspace_home = workspace.display().to_string();
    cmd.current_dir(workspace)
        .env_clear()
        .env("PATH", host::sandbox_path())
        .env("HOME", &workspace_home)
        .env("LANG", "C");
    #[cfg(windows)]
    {
        cmd.env("USERPROFILE", &workspace_home);
    }
    Ok(cmd)
}

/// Bubblewrap sandbox: the host filesystem is visible read-only, only the
/// workspace and a private /tmp are writable, and the network namespace is
/// unshared. Extra writable paths come from `BLORA_BWRAP_RW` (colon separated).
fn bwrap_command(workspace: &Path, command: &str) -> Result<Command> {
    let available = host::command("bwrap")
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
    let mut cmd = host::command("bwrap");
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
    let docker = host::command("docker")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if !docker.map(|status| status.success()).unwrap_or(false) {
        return Err(BloraError::Exec(
            "BLORA_EXEC=container requires a working docker CLI".to_owned(),
        ));
    }
    let mut cmd = host::command("docker");
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
    // `script` allocates a PTY on Unix. Windows has no equivalent on PATH,
    // so interactive commands run in the host shell without a console window.
    #[cfg(windows)]
    {
        return Ok(host::shell_command(workspace, command));
    }
    #[cfg(not(windows))]
    {
        let mut cmd = host::command("script");
        cmd.args(["-qefc", command, "/dev/null"])
            .current_dir(workspace);
        Ok(cmd)
    }
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
    let mut cmd = host::command("ssh");
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
        let program = cmd.get_program().to_string_lossy().to_ascii_lowercase();
        assert!(
            program == "unshare"
                || program == "sh"
                || program.ends_with("sh.exe")
                || program.ends_with("bash")
                || program.ends_with("bash.exe")
                || program.contains("cmd")
                || program.contains("powershell")
                || program.contains("pwsh"),
            "{program}"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn pty_uses_script() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = Isolation::Pty.shell_command(dir.path(), "echo hi").unwrap();
        assert_eq!(cmd.get_program(), "script");
    }

    #[cfg(windows)]
    #[test]
    fn pty_falls_back_to_the_host_shell() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = Isolation::Pty.shell_command(dir.path(), "echo hi").unwrap();
        let program = cmd.get_program().to_string_lossy().to_ascii_lowercase();
        assert!(
            program.contains("bash")
                || program.contains("sh")
                || program.contains("cmd")
                || program.contains("powershell")
                || program.contains("pwsh"),
            "{program}"
        );
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
