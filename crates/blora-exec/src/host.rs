// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Process launch details that differ between Unix and Windows.
//!
//! Unix always uses `sh -c`. Windows prefers Git Bash (`bash` or `sh` on
//! `PATH`, or `BLORA_SHELL`) so the same `cd && command` lines keep working,
//! and otherwise uses `cmd.exe /D /C`.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// How a command string is quoted and chained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostShell {
    Posix,
    PowerShell,
    Cmd,
}

struct ShellLaunch {
    kind: HostShell,
    program: String,
}

/// Build a process without a console window on Windows.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new(program);
        // CREATE_NO_WINDOW. Agent tools should not flash a console.
        cmd.creation_flags(0x0800_0000);
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new(program)
    }
}

pub fn host_shell() -> HostShell {
    launch().kind
}

/// Run `script` in the host shell with `workspace` as the working directory.
pub fn shell_command(workspace: &Path, script: &str) -> Command {
    let launch = launch();
    let mut cmd = command(&launch.program);
    match launch.kind {
        HostShell::Posix => {
            cmd.arg("-c").arg(script);
        }
        HostShell::PowerShell => {
            cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        }
        HostShell::Cmd => {
            cmd.args(["/D", "/C", script]);
        }
    }
    cmd.current_dir(workspace);
    cmd
}

/// Quote one path or argument for the host shell.
#[must_use]
pub fn quote(text: &str) -> String {
    quote_for(host_shell(), text)
}

fn quote_for(kind: HostShell, text: &str) -> String {
    match kind {
        HostShell::Posix => format!("'{}'", text.replace('\'', "'\\''")),
        HostShell::PowerShell => format!("'{}'", text.replace('\'', "''")),
        HostShell::Cmd => format!("\"{}\"", text.replace('"', "\"\"")),
    }
}

/// A command that changes directory and nothing else.
#[must_use]
pub fn cd_into(dest: &str) -> String {
    cd_into_for(host_shell(), dest)
}

fn cd_into_for(kind: HostShell, dest: &str) -> String {
    match kind {
        HostShell::Cmd => format!("cd /d {}", quote_for(kind, dest)),
        HostShell::Posix | HostShell::PowerShell => format!("cd {}", quote_for(kind, dest)),
    }
}

/// Run `command` after entering `cwd` when `cwd` is not the workspace root.
#[must_use]
pub fn chain(cwd: &str, command: &str) -> String {
    chain_for(host_shell(), cwd, command)
}

fn chain_for(kind: HostShell, cwd: &str, command: &str) -> String {
    if cwd == "." {
        return command.to_owned();
    }
    match kind {
        HostShell::PowerShell => format!(
            "Set-Location -LiteralPath {}; if (-not $?) {{ exit 1 }}; {command}",
            quote_for(kind, cwd)
        ),
        HostShell::Posix | HostShell::Cmd => {
            format!("{} && {command}", cd_into_for(kind, cwd))
        }
    }
}

/// `PATH` used when a sandbox drops the inherited environment.
#[must_use]
pub fn sandbox_path() -> String {
    #[cfg(windows)]
    {
        std::env::var("PATH").unwrap_or_else(|_| r"C:\Windows\System32".to_owned())
    }
    #[cfg(not(windows))]
    {
        "/usr/bin:/bin".to_owned()
    }
}

/// Stop a process spawned by Blora. Unix uses `kill`; Windows uses `taskkill /T /F`.
pub fn terminate_pid(pid: u32) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(windows)]
    {
        command("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    }
    #[cfg(not(windows))]
    {
        command("kill")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    }
}

/// Open `url` in the default browser. The first launcher that starts wins.
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = command("cmd")
            .args(["/D", "/C", "start", "", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(not(windows))]
    {
        for program in ["xdg-open", "open", "gio"] {
            if command(program)
                .arg(url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_ok()
            {
                return;
            }
        }
    }
}

/// Absolute for workspace containment: POSIX roots, UNC paths, and `C:\...`.
#[must_use]
pub fn path_is_absolute(target: &str) -> bool {
    let target = target.trim();
    if target.starts_with('/') || target.starts_with('\\') {
        return true;
    }
    let bytes = target.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn launch() -> &'static ShellLaunch {
    static LAUNCH: OnceLock<ShellLaunch> = OnceLock::new();
    LAUNCH.get_or_init(detect_launch)
}

fn detect_launch() -> ShellLaunch {
    #[cfg(not(windows))]
    {
        ShellLaunch {
            kind: HostShell::Posix,
            program: "sh".to_owned(),
        }
    }
    #[cfg(windows)]
    {
        if let Ok(value) = std::env::var("BLORA_SHELL") {
            let lower = value.to_ascii_lowercase();
            if lower == "cmd" || lower.ends_with("cmd.exe") {
                return ShellLaunch {
                    kind: HostShell::Cmd,
                    program: value,
                };
            }
            if lower.contains("powershell") || lower == "pwsh" || lower.ends_with("pwsh.exe") {
                return ShellLaunch {
                    kind: HostShell::PowerShell,
                    program: value,
                };
            }
            return ShellLaunch {
                kind: HostShell::Posix,
                program: value,
            };
        }
        if program_runs("bash") {
            return ShellLaunch {
                kind: HostShell::Posix,
                program: "bash".to_owned(),
            };
        }
        if program_runs("sh") {
            return ShellLaunch {
                kind: HostShell::Posix,
                program: "sh".to_owned(),
            };
        }
        ShellLaunch {
            kind: HostShell::Cmd,
            program: "cmd.exe".to_owned(),
        }
    }
}

#[cfg(windows)]
fn program_runs(program: &str) -> bool {
    command(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_shell_stays_posix() {
        #[cfg(not(windows))]
        assert_eq!(host_shell(), HostShell::Posix);
    }

    #[test]
    fn quotes_and_chains_for_posix_cmd_and_powershell() {
        assert_eq!(quote_for(HostShell::Posix, "it's"), "'it'\\''s'");
        assert_eq!(quote_for(HostShell::PowerShell, "it's"), "'it''s'");
        assert_eq!(quote_for(HostShell::Cmd, "a \"b\""), "\"a \"\"b\"\"\"");
        assert_eq!(chain_for(HostShell::Posix, "sub", "ls"), "cd 'sub' && ls");
        assert_eq!(
            chain_for(HostShell::Cmd, "sub", "dir"),
            "cd /d \"sub\" && dir"
        );
        assert_eq!(chain_for(HostShell::Posix, ".", "ls"), "ls");
        assert!(chain_for(HostShell::PowerShell, "sub", "ls").contains("Set-Location"));
    }

    #[test]
    fn drive_and_unc_paths_are_absolute() {
        assert!(path_is_absolute("/etc/passwd"));
        assert!(path_is_absolute(r"C:\Windows"));
        assert!(path_is_absolute("C:/Windows"));
        assert!(path_is_absolute(r"\\server\share"));
        assert!(path_is_absolute("D:"));
        assert!(!path_is_absolute("src/lib.rs"));
        assert!(!path_is_absolute(""));
    }
}
