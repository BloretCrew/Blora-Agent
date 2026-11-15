// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Explicit service dependencies for built-in tools.

use blora_imagine::{ImageEndpoint, MediaEndpoint};
use blora_web::{FetchOptions, SearchConfig};

/// Optional services used by built-in tools. No field implicitly reads process
/// environment; legacy callers construct this value during CLI bootstrap.
#[derive(Clone, Debug, Default)]
pub struct ToolServices {
    pub search: Option<SearchConfig>,
    pub fetch: Option<FetchOptions>,
    pub image: Option<ImageEndpoint>,
    pub media: Option<MediaEndpoint>,
    pub pty_enabled: bool,
    pub persist_tool_output: bool,
}

impl ToolServices {
    #[must_use]
    pub fn sdk_default() -> Self {
        Self {
            search: None,
            fetch: Some(FetchOptions {
                max_bytes: 2_000_000,
                timeout: std::time::Duration::from_secs(20),
                allow_private: false,
            }),
            image: None,
            media: None,
            pty_enabled: false,
            persist_tool_output: false,
        }
    }

    #[must_use]
    pub fn legacy() -> Self {
        Self::from_env()
    }

    #[must_use]
    pub fn from_env() -> Self {
        let fetch = FetchOptions::default();
        let search = SearchConfig::from_env().ok();
        let image = ImageEndpoint::from_env().ok();
        let media = MediaEndpoint::from_env();
        Self {
            search,
            fetch: Some(fetch),
            image,
            media,
            pty_enabled: std::env::var("BLORA_PTY").ok().as_deref() == Some("1"),
            persist_tool_output: true,
        }
    }
}
