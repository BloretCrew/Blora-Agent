// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_types::BloraError;

/// SDK-level error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SdkError {
    #[error(transparent)]
    Kernel(#[from] BloraError),
    #[error("configuration: {0}")]
    Config(String),
    #[error("a model provider is required; call .provider(...)")]
    ProviderRequired,
    #[error("the agent has been shut down")]
    Closed,
    #[error("session {0} already has a running foreground request")]
    RunActive(String),
    #[error("callback timed out: {0}")]
    CallbackTimeout(&'static str),
    #[error("run {0} timed out")]
    RunTimeout(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, SdkError>;
