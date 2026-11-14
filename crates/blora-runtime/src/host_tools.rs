// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Host-provided tools.
//!
//! The runtime owns tool declaration and dispatch; hosts supply named handlers
//! with owned JSON Schema metadata. This keeps the runtime independent of the
//! SDK facade while still allowing in-process extension.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use blora_content::ToolOutcome;
use blora_model::ToolDeclaration;
use blora_types::{BloraError, CancelToken, Result, RunId, SessionId};

/// Context passed to a host tool.
pub struct KernelToolContext {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub call_id: String,
    pub workspace: Option<PathBuf>,
    pub cancel: CancelToken,
}

/// A host tool failure. The message is shown to the model.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct KernelToolError {
    pub message: String,
    pub retryable: bool,
}

impl KernelToolError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }
}

/// A tool implemented by the embedding host.
pub trait KernelTool: Send + Sync {
    /// Provider-facing declaration. Names are owned so hosts are not limited
    /// to static strings.
    fn declaration(&self) -> ToolDeclaration;

    /// Read-only tools may run without approval and participate in
    /// read-only / plan-mode declarations.
    fn read_only(&self) -> bool {
        false
    }

    /// Whether the tool must be approved before execution.
    fn requires_approval(&self) -> bool {
        true
    }

    fn call(
        &self,
        ctx: &KernelToolContext,
        arguments: serde_json::Value,
    ) -> std::result::Result<ToolOutcome, KernelToolError>;
}

/// Registry of host tools. Duplicate names are rejected at registration.
#[derive(Default)]
pub struct HostToolRegistry {
    tools: BTreeMap<String, Arc<dyn KernelTool>>,
}

impl HostToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn KernelTool>) -> Result<()> {
        let name = tool.declaration().name;
        if name.trim().is_empty() {
            return Err(BloraError::Other("tool name is empty".to_owned()));
        }
        if self.tools.contains_key(&name) {
            return Err(BloraError::Other(format!(
                "host tool `{name}` is already registered"
            )));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<dyn KernelTool>> {
        self.tools.get(name).cloned()
    }

    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    #[must_use]
    pub fn is_read_only(&self, name: &str) -> bool {
        self.tools.get(name).is_some_and(|tool| tool.read_only())
    }

    #[must_use]
    pub fn declarations(&self) -> Vec<ToolDeclaration> {
        self.tools.values().map(|tool| tool.declaration()).collect()
    }
}
