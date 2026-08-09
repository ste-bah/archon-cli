//! Retry policy and error classification for [`super::RetryProvider`].
//!
//! This half answers "should we try again, and how long do we wait?" — it is
//! pure decision-making with no I/O and no knowledge of the provider being
//! wrapped. The retry loop that acts on these answers lives in the parent
//! module.
//!
//! See the parent module docstring for the spec references and the Phase-7
//! deviation note behind the `LlmError` mapping below.

use std::time::Duration;

use crate::provider::LlmError;

const MAX_INLINE_RATE_LIMIT_RETRY_SECS: u64 = 60;

/// Whether a mid-stream error is worth another attempt.
///
/// Mirrors the response-status classifier the providers already use; a decode
/// or connection fault mid-stream is the same transient class as one during
/// the handshake, it simply surfaces later.
///
/// `pub(super)` rather than private: the stream retry loop in the parent
/// module is the sole caller.
pub(super) fn stream_error_is_retryable(error_type: &str, message: &str) -> bool {
    let hay = format!("{error_type} {message}").to_ascii_lowercase();
    [
        "http_error",
        "decoding",
        "connection reset",
        "connection closed",
        "connection refused",
        "broken pipe",
        "timed out",
        "overloaded",
        "rate limit",
        "service unavailable",
        "upstream connect",
        "incomplete",
    ]
    .iter()
    .any(|marker| hay.contains(marker))
}

/// Configuration for `RetryProvider`'s backoff loop.
///
/// `max_attempts` is the *total* number of calls to `inner` per request,
/// including the first. The default of `3` matches ERR-PROV-02.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub multiplier: f64,
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(8),
            multiplier: 2.0,
            jitter: true,
        }
    }
}

/// Decision table for a single `LlmError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry,
    FailFast,
}

/// Classify an `LlmError` as retryable or persistent.
///
/// See the parent module docstring for the full mapping rationale.
pub fn classify(err: &LlmError) -> RetryDecision {
    match err {
        LlmError::Http(_) => RetryDecision::Retry,
        LlmError::RateLimited { retry_after_secs }
            if *retry_after_secs <= MAX_INLINE_RATE_LIMIT_RETRY_SECS =>
        {
            RetryDecision::Retry
        }
        LlmError::RateLimited { .. } => RetryDecision::FailFast,
        LlmError::Overloaded => RetryDecision::Retry,
        LlmError::Server { status, .. } if *status >= 500 => RetryDecision::Retry,

        LlmError::Auth(_)
        | LlmError::QuotaExceeded(_)
        | LlmError::Aborted
        | LlmError::Serialize(_)
        | LlmError::Unsupported(_)
        | LlmError::ContextWindowExceeded { .. }
        | LlmError::Server { .. }
        | LlmError::ProviderNotFound { .. } => RetryDecision::FailFast,
    }
}

/// Stable short code naming the error class, for runtime-supervisor events.
///
/// `pub(super)` rather than private: the retry loop in the parent module is
/// the sole caller.
pub(super) fn reason_code_for_error(err: &LlmError) -> &'static str {
    match err {
        LlmError::Http(_) => "http",
        LlmError::Auth(_) => "auth",
        LlmError::RateLimited { .. } => "rate_limited",
        LlmError::Overloaded => "overloaded",
        LlmError::Server { .. } => "server",
        LlmError::Serialize(_) => "serialize",
        LlmError::Unsupported(_) => "unsupported",
        LlmError::ProviderNotFound { .. } => "provider_not_found",
        LlmError::QuotaExceeded(_) => "quota_exceeded",
        LlmError::Aborted => "aborted",
        LlmError::ContextWindowExceeded { .. } => "context_window_exceeded",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_rate_limit_is_retryable() {
        let err = LlmError::RateLimited {
            retry_after_secs: 30,
        };

        assert_eq!(classify(&err), RetryDecision::Retry);
    }

    #[test]
    fn long_rate_limit_is_fail_fast() {
        let err = LlmError::RateLimited {
            retry_after_secs: 8_004,
        };

        assert_eq!(classify(&err), RetryDecision::FailFast);
    }
}
