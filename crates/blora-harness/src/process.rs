// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use crate::{HarnessError, HarnessEvent, Outcome, Result, RunRequest};
use blora_types::CancelToken;
use std::ffi::OsString;
use std::path::Path;

#[cfg(unix)]
pub(crate) fn execute(
    path: &Path,
    request: &RunRequest,
    args: &[OsString],
    cancel: &CancelToken,
    mut callback: impl FnMut(HarnessEvent),
) -> Result<Outcome> {
    use crate::protocol::Parser;
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let kill = crate::detect_in_executable("kill").ok_or_else(|| {
        HarnessError::Process("kill executable required for process-group cleanup".into())
    })?;
    let child = Command::new(path)
        .args(args)
        .current_dir(&request.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let mut guard = ProcessGuard { child, kill };
    let mut stdout = guard.child.stdout.take().expect("piped stdout");
    let mut stderr = guard.child.stderr.take().expect("piped stderr");
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let mut parser = Parser::new(request.kind);
    let mut pending = Vec::new();
    let mut diagnostic = Vec::new();
    let mut total = 0usize;
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let mut exited = None;
    let mut ended_at = None;
    loop {
        if cancel.is_cancelled() {
            return Err(HarnessError::Cancelled);
        }
        let mut buffer = [0u8; 8192];
        // Limit work per iteration so a flooding CLI cannot starve cancellation.
        for _ in 0..8 {
            match stdout.read(&mut buffer) {
                Ok(0) => {
                    stdout_eof = true;
                    break;
                }
                Ok(n) => {
                    total += n;
                    if total > 64 * 1024 * 1024 {
                        return Err(HarnessError::Protocol("stdout exceeded 64 MiB".into()));
                    }
                    for byte in &buffer[..n] {
                        if *byte == b'\n' {
                            parser.line(&pending, &mut callback)?;
                            pending.clear();
                        } else {
                            pending.push(*byte);
                            if pending.len() > 1024 * 1024 {
                                return Err(HarnessError::Protocol(
                                    "JSON line exceeded 1 MiB".into(),
                                ));
                            }
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.into()),
            }
        }
        for _ in 0..8 {
            match stderr.read(&mut buffer) {
                Ok(0) => {
                    stderr_eof = true;
                    break;
                }
                Ok(n) => {
                    let available = (32 * 1024usize).saturating_sub(diagnostic.len());
                    diagnostic.extend_from_slice(&buffer[..n.min(available)]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.into()),
            }
        }
        if exited.is_none() {
            exited = guard.child.try_wait()?;
            if exited.is_some() {
                ended_at = Some(Instant::now());
                // Descendants may retain pipes after the leader exits.
                guard.kill_group();
            }
        }
        if let Some(status) = exited {
            if stdout_eof && stderr_eof
                || ended_at.is_some_and(|t| t.elapsed() > Duration::from_millis(500))
            {
                if !pending.is_empty() {
                    parser.line(&pending, &mut callback)?;
                }
                if !status.success() {
                    // stderr can contain secrets; deliberately keep it out of UI/log errors.
                    return Err(HarnessError::Process(format!(
                        "{} exited with {status}; check its external authentication/configuration ({} stderr bytes captured)",
                        request.kind,
                        diagnostic.len()
                    )));
                }
                return parser.finish();
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn nonblocking(fd: &impl std::os::fd::AsFd) -> std::io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd)?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?;
    Ok(())
}

#[cfg(unix)]
struct ProcessGuard {
    child: std::process::Child,
    kill: std::path::PathBuf,
}

#[cfg(unix)]
impl ProcessGuard {
    fn kill_group(&mut self) {
        let _ = std::process::Command::new(&self.kill)
            .args(["-KILL", "--", &format!("-{}", self.child.id())])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(unix)]
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.kill_group();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(not(unix))]
pub(crate) fn execute(
    _: &Path,
    _: &RunRequest,
    _: &[OsString],
    _: &CancelToken,
    _: impl FnMut(HarnessEvent),
) -> Result<Outcome> {
    Err(HarnessError::Process(
        "external execution requires Unix process-group cleanup on this platform".into(),
    ))
}
