// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Background process tracking with captured output.
//!
//! Every background shell gets a durable log file under
//! `.blora/processes/<id>.log`. The log is the source of truth for
//! `process output`; the in-memory record only keeps live handles.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use blora_types::{BloraError, Result};

use crate::LocalBackend;
use crate::host::terminate_pid;

const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// A read-only view of one tracked process.
#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub id: String,
    pub pid: u32,
    pub command: String,
    pub status: String,
    pub log_path: String,
}

#[derive(Debug)]
struct ProcessRecord {
    id: String,
    pid: u32,
    command: String,
    status: String,
    log_path: PathBuf,
    stdin: Option<ChildStdin>,
    exit_code: Option<i32>,
}

type SharedRecord = Arc<Mutex<ProcessRecord>>;

fn registry() -> &'static Mutex<BTreeMap<String, SharedRecord>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<String, SharedRecord>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn lock_registry() -> MutexGuard<'static, BTreeMap<String, SharedRecord>> {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn record_for(key: &str) -> Option<SharedRecord> {
    let registry = lock_registry();
    if let Some(record) = registry.get(key) {
        return Some(record.clone());
    }
    registry
        .values()
        .find(|record| {
            record
                .lock()
                .map(|r| r.pid.to_string() == key)
                .unwrap_or(false)
        })
        .cloned()
}

fn process_key(pid: u32) -> String {
    format!("proc_{pid}")
}

pub fn process_infos() -> Vec<ProcessInfo> {
    let registry = lock_registry();
    registry
        .values()
        .filter_map(|record| record.lock().ok().map(|record| format_info(&record)))
        .collect()
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
        let log_dir = self.policy().workspace().join(".blora/processes");
        std::fs::create_dir_all(&log_dir).map_err(BloraError::exec)?;
        let mut child = self
            .isolation()
            .shell_command(self.policy().workspace(), command)?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(BloraError::exec)?;
        let pid = child.id();
        let id = process_key(pid);
        let log_path = log_dir.join(format!("{id}.log"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(BloraError::exec)?;
        let log = Arc::new(Mutex::new(file));
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let record = Arc::new(Mutex::new(ProcessRecord {
            id: id.clone(),
            pid,
            command: command.to_owned(),
            status: "running".to_owned(),
            log_path: log_path.clone(),
            stdin,
            exit_code: None,
        }));
        lock_registry().insert(id.clone(), record.clone());
        if let Some(stdout) = stdout {
            let log = log.clone();
            thread::spawn(move || pump(stdout, log, None));
        }
        if let Some(stderr) = stderr {
            let log = log.clone();
            thread::spawn(move || pump(stderr, log, Some("[stderr] ")));
        }
        let waiter = record.clone();
        thread::spawn(move || {
            let status = child.wait();
            if let Ok(mut record) = waiter.lock() {
                match status {
                    Ok(code) => {
                        record.exit_code = code.code();
                        record.status = if code.success() {
                            "exited".to_owned()
                        } else {
                            format!("exit {}", code.code().unwrap_or(-1))
                        };
                    }
                    Err(err) => record.status = format!("error {err}"),
                }
            }
        });
        Ok(format!(
            "started process {id} (pid {pid}); output: {}",
            log_path.display()
        ))
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
            .filter_map(|record| record.lock().ok().map(|record| format_info(&record)))
            .map(|info| {
                format!(
                    "{}  pid {}  {}  {}\n  log: {}",
                    info.id, info.pid, info.status, info.command, info.log_path
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// Read output appended after `cursor`. The returned cursor is the next
    /// byte offset.
    pub fn process_output(
        &self,
        key: &str,
        cursor: usize,
        max_bytes: usize,
    ) -> Result<(String, usize, String)> {
        self.policy()
            .require(self.policy().file_read(), "process_output")?;
        let record =
            record_for(key).ok_or_else(|| BloraError::Policy(format!("unknown process: {key}")))?;
        let (path, status, pid) = {
            let record = record
                .lock()
                .map_err(|_| BloraError::Exec("process record is poisoned".to_owned()))?;
            (record.log_path.clone(), record.status.clone(), record.pid)
        };
        let bytes = std::fs::read(&path).unwrap_or_default();
        let start = cursor.min(bytes.len());
        let end = start
            .saturating_add(max_bytes.clamp(1, MAX_OUTPUT_BYTES))
            .min(bytes.len());
        let text = String::from_utf8_lossy(&bytes[start..end]).into_owned();
        Ok((
            format!("process {key} (pid {pid}) status={status}\n{text}"),
            end,
            status,
        ))
    }

    /// Wait for a process to exit, or return its current status on timeout.
    pub fn process_wait(&self, key: &str, timeout: Duration) -> Result<String> {
        self.policy()
            .require(self.policy().file_read(), "process_wait")?;
        let record =
            record_for(key).ok_or_else(|| BloraError::Policy(format!("unknown process: {key}")))?;
        let deadline = Instant::now() + timeout;
        loop {
            let status = record
                .lock()
                .map_err(|_| BloraError::Exec("process record is poisoned".to_owned()))?
                .status
                .clone();
            if status != "running" {
                return Ok(format!("process {key} {status}"));
            }
            if Instant::now() >= deadline {
                return Ok(format!("process {key} still running (timeout)"));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Write to a process's stdin. `close` drops the pipe after writing.
    pub fn process_stdin(&self, key: &str, data: &str, close: bool) -> Result<String> {
        self.policy()
            .require(self.policy().shell(), &format!("process_stdin {key}"))?;
        let record =
            record_for(key).ok_or_else(|| BloraError::Policy(format!("unknown process: {key}")))?;
        let mut record = record
            .lock()
            .map_err(|_| BloraError::Exec("process record is poisoned".to_owned()))?;
        let stdin = record
            .stdin
            .as_mut()
            .ok_or_else(|| BloraError::Exec(format!("process {key} stdin is closed")))?;
        stdin.write_all(data.as_bytes()).map_err(BloraError::exec)?;
        stdin.flush().map_err(BloraError::exec)?;
        if close {
            record.stdin.take();
        }
        Ok(format!("wrote {} bytes to {key}", data.len()))
    }

    pub fn process_kill(&self, key: &str) -> Result<String> {
        self.policy()
            .require(self.policy().shell(), &format!("process_kill {key}"))?;
        let record =
            record_for(key).ok_or_else(|| BloraError::Policy(format!("unknown process: {key}")))?;
        let pid = record
            .lock()
            .map_err(|_| BloraError::Exec("process record is poisoned".to_owned()))?
            .pid;
        let status = terminate_pid(pid).map_err(BloraError::exec)?;
        if let Ok(mut record) = record.lock() {
            record.status = "killed".to_owned();
        }
        if status.success() {
            Ok(format!("killed {key} (pid {pid})"))
        } else {
            Err(BloraError::Exec(format!("kill {key} failed")))
        }
    }
}

fn pump(mut reader: impl Read, log: Arc<Mutex<File>>, prefix: Option<&'static str>) {
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if let Ok(mut log) = log.lock() {
                    if let Some(prefix) = prefix {
                        let _ = log.write_all(prefix.as_bytes());
                    }
                    let _ = log.write_all(&buffer[..read]);
                    let _ = log.flush();
                }
            }
            Err(_) => break,
        }
    }
}

fn format_info(record: &ProcessRecord) -> ProcessInfo {
    ProcessInfo {
        id: record.id.clone(),
        pid: record.pid,
        command: record.command.clone(),
        status: record.status.clone(),
        log_path: record.log_path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::LocalBackend;
    use blora_policy::Policy;
    use std::time::Duration;

    #[test]
    fn captures_output_and_waits() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::new(dir.path(), true).unwrap();
        let backend = LocalBackend::new(policy);
        let started = backend
            .shell_background("printf 'one'; sleep 0.1; printf 'two'")
            .unwrap();
        let key = started
            .split_whitespace()
            .nth(2)
            .expect("process id")
            .to_owned();
        let waited = backend.process_wait(&key, Duration::from_secs(5)).unwrap();
        assert!(waited.contains("exited"), "{waited}");
        let (output, cursor, _) = backend.process_output(&key, 0, 64 * 1024).unwrap();
        assert!(output.contains("one"), "{output}");
        assert!(output.contains("two"), "{output}");
        let (_, next, _) = backend.process_output(&key, cursor, 64).unwrap();
        assert_eq!(cursor, next);
    }

    #[test]
    fn lists_and_kills() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::new(dir.path(), true).unwrap();
        let backend = LocalBackend::new(policy);
        let started = backend.shell_background("sleep 5").unwrap();
        assert!(started.contains("started process"));
        let key = started.split_whitespace().nth(2).unwrap().to_owned();
        assert!(backend.process_list().unwrap().contains(&key));
        backend.process_kill(&key).unwrap();
        assert!(
            backend
                .process_wait(&key, Duration::from_secs(2))
                .unwrap()
                .contains("killed")
        );
    }
}
