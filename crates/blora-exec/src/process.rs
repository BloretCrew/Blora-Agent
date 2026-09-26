// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;

use blora_types::{BloraError, Result};

use crate::LocalBackend;

#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub command: String,
    pub status: String,
}

fn registry() -> &'static Mutex<BTreeMap<u32, ProcessInfo>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<u32, ProcessInfo>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn lock_registry() -> std::sync::MutexGuard<'static, BTreeMap<u32, ProcessInfo>> {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl LocalBackend {
    pub fn shell_background(&self, command: &str) -> Result<String> {
        self.policy().require(
            self.policy().shell_command(command),
            &format!("shell {command}"),
        )?;
        self.spawn_background_unchecked(command)
    }

    pub(crate) fn spawn_background_unchecked(&self, command: &str) -> Result<String> {
        if command.trim().is_empty() {
            return Err(BloraError::Exec("empty command".to_owned()));
        }
        let mut child = self
            .isolation()
            .shell_command(self.policy().workspace(), command)?
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(BloraError::exec)?;
        let pid = child.id();
        lock_registry().insert(
            pid,
            ProcessInfo {
                pid,
                command: command.to_owned(),
                status: "running".to_owned(),
            },
        );
        thread::spawn(move || {
            let status = child.wait();
            let mut registry = lock_registry();
            if let Some(info) = registry.get_mut(&pid) {
                info.status = match status {
                    Ok(code) if code.success() => "exited".to_owned(),
                    Ok(code) => format!("exit {}", code.code().unwrap_or(-1)),
                    Err(err) => format!("error {err}"),
                };
            }
        });
        Ok(format!("started pid {pid}"))
    }

    pub fn process_list(&self) -> Result<String> {
        self.policy()
            .require(self.policy().file_read(), "process_list")?;
        let registry = lock_registry();
        if registry.is_empty() {
            return Ok("(no tracked processes)".to_owned());
        }
        Ok(registry
            .values()
            .map(|info| format!("{}  {}  {}", info.pid, info.status, info.command))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn process_kill(&self, pid: u32) -> Result<String> {
        self.policy()
            .require(self.policy().shell(), &format!("process_kill {pid}"))?;
        let tracked = lock_registry().contains_key(&pid);
        if !tracked {
            return Err(BloraError::Policy(format!(
                "pid {pid} is not a Blora-tracked process"
            )));
        }
        let status = Command::new("kill")
            .arg(pid.to_string())
            .status()
            .map_err(BloraError::exec)?;
        if let Some(info) = lock_registry().get_mut(&pid) {
            info.status = "killed".to_owned();
        }
        if status.success() {
            Ok(format!("killed {pid}"))
        } else {
            Err(BloraError::Exec(format!("kill {pid} failed")))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::LocalBackend;
    use blora_policy::Policy;

    #[test]
    fn tracks_background_process() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::new(dir.path(), true).unwrap();
        let backend = LocalBackend::new(policy);
        let started = backend.shell_background("true").unwrap();
        assert!(started.contains("started pid"));
        assert!(backend.process_list().unwrap().contains("true"));
    }
}
