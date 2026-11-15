// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use blora_runtime::HostToolRegistry;
pub use blora_storage::RuntimeStore as Store;
use blora_types::{BloraError, CancelToken, SessionId};
use serde_json::Value;

use crate::error::{Result, SdkError};

/// Web/chat boundary: denies filesystem, shell, and process capabilities.
///
/// This is a defense-in-depth backend for hosts which want to expose built-in
/// read-only tools while ensuring write and process tools are never executed.
pub struct ReadOnlyBackend {
    workspace: std::path::PathBuf,
    allowed_roots: Vec<std::path::PathBuf>,
    network_enabled: bool,
}

impl ReadOnlyBackend {
    pub fn new(
        workspace: impl Into<std::path::PathBuf>,
        allowed_roots: Vec<std::path::PathBuf>,
        network_enabled: bool,
    ) -> Result<Self> {
        let workspace = workspace.into();
        let workspace = workspace.canonicalize().map_err(BloraError::exec)?;
        let mut roots = Vec::new();
        for root in allowed_roots {
            roots.push(root.canonicalize().map_err(BloraError::exec)?);
        }
        Ok(Self {
            workspace,
            allowed_roots: roots,
            network_enabled,
        })
    }

    fn ensure_read_path(&self, path: &str) -> Result<std::path::PathBuf> {
        if path.starts_with('/')
            || path.starts_with('\\')
            || path.chars().nth(1) == Some(':')
            || path
                .split(['/', '\\'])
                .any(|part| part == ".." || part.is_empty())
        {
            return Err(SdkError::Unsupported(
                "path must be relative and confined".to_owned(),
            ));
        }
        let candidate = self.workspace.join(path);
        let resolved = candidate.canonicalize().map_err(BloraError::exec)?;
        let metadata = std::fs::metadata(&resolved).map_err(BloraError::exec)?;
        if !metadata.is_file() {
            return Err(SdkError::Unsupported(
                "read_file only accepts regular files".to_owned(),
            ));
        }
        if !resolved.starts_with(&self.workspace)
            && !self
                .allowed_roots
                .iter()
                .any(|root| resolved.starts_with(root))
        {
            return Err(SdkError::Unsupported(
                "path is outside allowed roots".to_owned(),
            ));
        }
        Ok(resolved)
    }

    pub fn read_file(&self, path: &str) -> Result<String> {
        let resolved = self.ensure_read_path(path)?;
        Ok(std::fs::read_to_string(resolved).map_err(BloraError::exec)?)
    }

    pub fn list_dir(&self, path: &str) -> Result<Vec<String>> {
        let resolved = if path == "." || path.is_empty() {
            self.workspace.clone()
        } else {
            let target = self.workspace.join(path);
            let resolved = target.canonicalize().map_err(BloraError::exec)?;
            if !resolved.is_dir()
                || (!resolved.starts_with(&self.workspace)
                    && !self
                        .allowed_roots
                        .iter()
                        .any(|root| resolved.starts_with(root)))
            {
                return Err(SdkError::Unsupported(
                    "directory is outside allowed roots".to_owned(),
                ));
            }
            resolved
        };
        let mut items = std::fs::read_dir(resolved)
            .map_err(BloraError::exec)?
            .map(|entry| {
                entry
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .map_err(BloraError::exec)
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        items.sort();
        Ok(items)
    }

    pub fn execute_named(
        &self,
        registry: &HostToolRegistry,
        name: &str,
        arguments: Value,
        session_id: SessionId,
        run_id: blora_types::RunId,
        call_id: String,
        cancel: CancelToken,
    ) -> Result<blora_content::ToolOutcome> {
        if let Some(tool) = registry.get(name) {
            if !tool.read_only() {
                return Err(SdkError::Unsupported(format!(
                    "host tool `{name}` is not read-only"
                )));
            }
            let context = blora_runtime::KernelToolContext {
                session_id,
                run_id,
                call_id,
                workspace: Some(self.workspace.clone()),
                cancel,
            };
            return tool
                .call(&context, arguments)
                .map_err(|error| SdkError::Internal(error.to_string()));
        }
        match name {
            "read_file" => {
                let path = arguments
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| SdkError::Config("read_file requires path".to_owned()))?;
                Ok(blora_content::ToolOutcome::text(self.read_file(path)?))
            }
            "list_dir" => {
                let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
                Ok(blora_content::ToolOutcome::text(
                    self.list_dir(path)?.join("\n"),
                ))
            }
            "web_fetch" if self.network_enabled => {
                let url = arguments
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| SdkError::Config("web_fetch requires url".to_owned()))?;
                let page = blora_web::fetch(
                    url,
                    &blora_web::FetchOptions {
                        allow_private: false,
                        ..blora_web::FetchOptions::default()
                    },
                )
                .map_err(SdkError::Kernel)?;
                Ok(blora_content::ToolOutcome::text(page.text))
            }
            "web_fetch" | "web_search" if !self.network_enabled => Err(SdkError::Unsupported(
                "network access is disabled for this profile".to_owned(),
            )),
            _ => Err(SdkError::Unsupported(format!(
                "tool `{name}` is not enabled by ReadOnlyBackend"
            ))),
        }
    }
}

/// Cancellation-aware polling helper for host implementations.
pub fn check_cancel(cancel: &CancelToken) -> Result<()> {
    if cancel.is_cancelled() {
        Err(SdkError::Kernel(BloraError::Cancelled))
    } else {
        Ok(())
    }
}

/// Event listener used by simple synchronous hosts.
pub fn listen(
    runtime: &blora_runtime::Runtime,
    session: &SessionId,
    after: u64,
    mut callback: impl FnMut(&blora_events::EventEnvelope),
) -> Result<()> {
    let rx = runtime.subscribe_session(session);
    let mut cursor = after;
    loop {
        for event in runtime.events_after(session, cursor)? {
            cursor = event.sequence;
            callback(&event);
            if matches!(
                event.event_type.as_str(),
                "run.completed" | "run.failed" | "run.cancelled"
            ) {
                return Ok(());
            }
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

/// Minimal fakeable in-memory host-tool router.
#[derive(Default)]
pub struct HostToolTable {
    names: Mutex<HashSet<String>>,
}

impl HostToolTable {
    pub fn reserve(&self, name: impl Into<String>) -> Result<()> {
        let name = name.into();
        let mut names = self
            .names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !names.insert(name.clone()) {
            return Err(SdkError::Config(format!("duplicate tool `{name}`")));
        }
        Ok(())
    }
}

/// Small helper for host tools which return a text result.
pub fn text_tool_result(text: impl Into<String>) -> blora_content::ToolOutcome {
    blora_content::ToolOutcome::text(text)
}

/// The JSON shape used for host tool result metadata.
pub fn tool_metadata(extra: Value) -> blora_content::ToolMetadata {
    blora_content::ToolMetadata {
        extra,
        ..Default::default()
    }
}
