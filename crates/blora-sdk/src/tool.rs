// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::Arc;

use blora_content::ToolOutcome;
use blora_model::ToolDeclaration;
use blora_runtime::{KernelTool, KernelToolContext, KernelToolError};
use blora_types::{CancelToken, RunId, SessionId};
use serde_json::Value;

/// Owned metadata for a host tool.
#[derive(Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub read_only: bool,
    pub requires_approval: bool,
}

impl ToolDefinition {
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            read_only: false,
            requires_approval: true,
        }
    }

    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self.requires_approval = false;
        self
    }
}

/// Context passed to an SDK tool implementation.
#[derive(Clone, Debug)]
pub struct ToolCallContext {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub call_id: String,
    pub cancel: CancelToken,
}

/// Tool failure shown to the model.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ToolError {
    pub message: String,
    pub retryable: bool,
}

impl ToolError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }
}

/// A tool implemented by the embedding host.
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    fn call(
        &self,
        ctx: &ToolCallContext,
        arguments: Value,
    ) -> std::result::Result<ToolOutcome, ToolError>;
}

pub(crate) struct ToolAdapter(pub Arc<dyn Tool>);

impl KernelTool for ToolAdapter {
    fn declaration(&self) -> ToolDeclaration {
        let definition = self.0.definition();
        ToolDeclaration {
            name: definition.name,
            description: definition.description,
            parameters: definition.parameters,
        }
    }

    fn read_only(&self) -> bool {
        self.0.definition().read_only
    }

    fn requires_approval(&self) -> bool {
        self.0.definition().requires_approval
    }

    fn call(
        &self,
        ctx: &KernelToolContext,
        arguments: Value,
    ) -> std::result::Result<ToolOutcome, KernelToolError> {
        let context = ToolCallContext {
            session_id: ctx.session_id.clone(),
            run_id: ctx.run_id.clone(),
            call_id: ctx.call_id.clone(),
            cancel: ctx.cancel.clone(),
        };
        self.0
            .call(&context, arguments)
            .map_err(|error| KernelToolError {
                message: error.message,
                retryable: error.retryable,
            })
    }
}

/// Closure-backed tool convenience.
pub struct FnTool<F> {
    definition: ToolDefinition,
    handler: F,
}

impl<F> FnTool<F> {
    pub fn new(definition: ToolDefinition, handler: F) -> Self {
        Self {
            definition,
            handler,
        }
    }
}

impl<F> Tool for FnTool<F>
where
    F: Fn(&ToolCallContext, Value) -> std::result::Result<ToolOutcome, ToolError> + Send + Sync,
{
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn call(
        &self,
        ctx: &ToolCallContext,
        arguments: Value,
    ) -> std::result::Result<ToolOutcome, ToolError> {
        (self.handler)(ctx, arguments)
    }
}

/// Build a closure-backed tool.
pub fn tool<F>(
    name: impl Into<String>,
    description: impl Into<String>,
    parameters: Value,
    read_only: bool,
    handler: F,
) -> Arc<dyn Tool>
where
    F: Fn(&ToolCallContext, Value) -> std::result::Result<ToolOutcome, ToolError>
        + Send
        + Sync
        + 'static,
{
    let mut definition = ToolDefinition::new(name, description, parameters);
    if read_only {
        definition = definition.read_only();
    }
    Arc::new(FnTool::new(definition, handler))
}
