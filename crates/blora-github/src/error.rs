// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::fmt;

/// Recoverable failure while handling a GitHub event.
#[derive(Debug, Clone)]
pub struct GithubError {
    pub message: String,
}

impl GithubError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for GithubError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GithubError {}

pub type Result<T> = std::result::Result<T, GithubError>;
