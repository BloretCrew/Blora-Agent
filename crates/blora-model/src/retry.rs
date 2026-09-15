// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Retry classification and exponential backoff with jitter.
//!
//! Rules follow the behaviour that has converged across production harnesses:
//! 429 and 5xx retry, 4xx client errors are fatal, `Retry-After` wins over the
//! local curve, delays are capped, and every wait carries jitter.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use blora_types::BloraError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryClass {
    /// Transient failure; retry the same provider after a backoff.
    Retry,
    /// Rate limited; retry, honouring `Retry-After` when present.
    RateLimited,
    /// Do not retry this provider. Authentication and validation errors land here.
    Fatal,
}

const BASE_DELAY_MS: u64 = 1_000;
const MAX_DELAY_MS: u64 = 30_000;
const MAX_RETRY_AFTER_SECS: u64 = 120;

/// Number of retries per provider before falling through to the next one.
#[must_use]
pub fn max_retries() -> u32 {
    std::env::var("BLORA_MAX_RETRIES")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(5)
        .min(100)
}

#[must_use]
pub fn http_status(err: &BloraError) -> Option<u16> {
    let text = err.to_string();
    let rest = text.split("HTTP ").nth(1)?;
    rest.split(|ch: char| !ch.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse::<u16>().ok())
}

#[must_use]
pub fn retry_after_secs(err: &BloraError) -> Option<u64> {
    let text = err.to_string();
    let rest = text.split("retry-after=").nth(1)?;
    rest.split(|ch: char| !ch.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse::<u64>().ok())
}

#[must_use]
pub fn classify(err: &BloraError) -> RetryClass {
    match err {
        BloraError::Cancelled => return RetryClass::Fatal,
        BloraError::Provider(_) => {}
        _ => return RetryClass::Fatal,
    }
    if let Some(status) = http_status(err) {
        return match status {
            429 => RetryClass::RateLimited,
            408 | 409 | 425 | 500 | 502 | 503 | 504 | 520 | 522 | 524 | 529 => RetryClass::Retry,
            _ => RetryClass::Fatal,
        };
    }
    let text = err.to_string().to_ascii_lowercase();
    if text.contains("transport:")
        || text.contains("timed out")
        || text.contains("timeout")
        || text.contains("connection")
        || text.contains("stream ended before")
        || text.contains("temporar")
        || text.contains("overloaded")
    {
        RetryClass::Retry
    } else {
        RetryClass::Fatal
    }
}

/// Delay before the given attempt (0-based). `Retry-After` overrides the curve.
#[must_use]
pub fn backoff_delay(attempt: u32, retry_after: Option<u64>) -> Duration {
    if let Some(secs) = retry_after {
        return Duration::from_secs(secs.min(MAX_RETRY_AFTER_SECS));
    }
    let exp = BASE_DELAY_MS.saturating_mul(1u64 << attempt.min(10));
    let capped = exp.min(MAX_DELAY_MS);
    // ±20% jitter from a cheap time-seeded generator; determinism is not needed here.
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(7);
    let jitter_pct = (seed.wrapping_mul(6_364_136_223_846_793_005) >> 33) % 41; // 0..=40
    let factor = 80 + jitter_pct; // 80..=120
    Duration::from_millis(capped * factor / 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_status_codes() {
        let limited = BloraError::provider("HTTP 429 retry-after=7: slow down");
        assert_eq!(classify(&limited), RetryClass::RateLimited);
        assert_eq!(retry_after_secs(&limited), Some(7));
        assert_eq!(http_status(&limited), Some(429));
        assert_eq!(
            classify(&BloraError::provider("HTTP 503: busy")),
            RetryClass::Retry
        );
        assert_eq!(
            classify(&BloraError::provider("HTTP 401: nope")),
            RetryClass::Fatal
        );
        assert_eq!(
            classify(&BloraError::provider("transport: connection reset")),
            RetryClass::Retry
        );
        assert_eq!(classify(&BloraError::Cancelled), RetryClass::Fatal);
    }

    #[test]
    fn backoff_is_capped_and_honours_retry_after() {
        assert_eq!(backoff_delay(3, Some(9)), Duration::from_secs(9));
        assert_eq!(backoff_delay(0, Some(999)), Duration::from_secs(120));
        let big = backoff_delay(20, None);
        assert!(big <= Duration::from_millis(36_000));
        let small = backoff_delay(0, None);
        assert!(small >= Duration::from_millis(800) && small <= Duration::from_millis(1_200));
    }
}
