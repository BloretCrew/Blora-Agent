// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use thiserror::Error;

pub type Result<T> = std::result::Result<T, BloraError>;

#[derive(Debug, Error)]
pub enum BloraError {
    #[error("invalid identifier: {0}")]
    InvalidId(String),
    #[error("illegal state transition from {from} to {to}")]
    IllegalTransition { from: String, to: String },
    #[error("event sequence mismatch: expected {expected}, got {actual}")]
    SequenceMismatch { expected: u64, actual: u64 },
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("run not found: {0}")]
    RunNotFound(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("event error: {0}")]
    Event(String),
    #[error("run cancelled")]
    Cancelled,
    #[error("policy denied: {0}")]
    Policy(String),
    #[error("approval required: {0}")]
    ApprovalRequired(String),
    #[error("execution error: {0}")]
    Exec(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("{0}")]
    Other(String),
}

impl BloraError {
    pub fn storage(err: impl ToString) -> Self {
        Self::Storage(err.to_string())
    }

    pub fn event(err: impl ToString) -> Self {
        Self::Event(err.to_string())
    }

    pub fn exec(err: impl ToString) -> Self {
        Self::Exec(err.to_string())
    }

    pub fn provider(err: impl ToString) -> Self {
        Self::Provider(err.to_string())
    }
}
