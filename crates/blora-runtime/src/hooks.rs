// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Run an optional executable from `BLORA_HOOKS_DIR`. Hook failure never aborts the run.
pub fn fire(name: &str, payload: &str) {
    let Ok(dir) = std::env::var("BLORA_HOOKS_DIR") else {
        return;
    };
    let path = std::path::Path::new(&dir).join(name);
    if !path.is_file() {
        return;
    }
    let mut child = match Command::new(&path)
        .env("BLORA_HOOK", name)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            tracing::warn!("hook {name} failed to start: {err}");
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
    }
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait());
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(status)) if !status.success() => {
            tracing::warn!("hook {name} exited {}", status.code().unwrap_or(-1));
        }
        Ok(Err(err)) => tracing::warn!("hook {name} wait failed: {err}"),
        Err(_) => {
            let _ = Command::new("kill").arg(pid.to_string()).status();
            tracing::warn!("hook {name} timed out");
        }
        Ok(Ok(_)) => {}
    }
}
