// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Shared HTTP plumbing for provider adapters: idle watchdog, structured errors.
//!
//! Provider errors are encoded as `HTTP <status>[ retry-after=<secs>]: <body>` so the
//! retry layer can classify them without a provider-specific error type.

use std::time::Duration;

use blora_types::BloraError;

const DEFAULT_IDLE_SECS: u64 = 300;
const DEFAULT_CONNECT_SECS: u64 = 30;
const MAX_BODY_CHARS: usize = 600;

/// Build a ureq agent whose socket read timeout doubles as a stream idle watchdog.
/// `BLORA_STREAM_IDLE_SECS` overrides the default of 300 seconds.
#[must_use]
pub fn agent() -> ureq::Agent {
    let idle = std::env::var("BLORA_STREAM_IDLE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_IDLE_SECS);
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(DEFAULT_CONNECT_SECS))
        .timeout_read(Duration::from_secs(idle))
        .timeout_write(Duration::from_secs(DEFAULT_CONNECT_SECS))
        .build()
}

/// Convert a ureq failure into a classifiable provider error.
#[must_use]
pub fn map_error(err: ureq::Error) -> BloraError {
    match err {
        ureq::Error::Status(code, response) => {
            let retry_after = response
                .header("retry-after")
                .and_then(|value| value.trim().parse::<u64>().ok());
            let body = response.into_string().unwrap_or_default();
            let body: String = body.chars().take(MAX_BODY_CHARS).collect();
            match retry_after {
                Some(secs) => {
                    BloraError::provider(format!("HTTP {code} retry-after={secs}: {}", body.trim()))
                }
                None => BloraError::provider(format!("HTTP {code}: {}", body.trim())),
            }
        }
        ureq::Error::Transport(transport) => {
            BloraError::provider(format!("transport: {transport}"))
        }
    }
}

/// Error used when a stream closes before any terminal event and without content.
#[must_use]
pub fn incomplete_stream(what: &str) -> BloraError {
    BloraError::provider(format!("stream ended before {what}"))
}

/// Default output budget for a single completion.
#[must_use]
pub fn max_output_tokens() -> u32 {
    std::env::var("BLORA_MAX_OUTPUT_TOKENS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(8192)
}
