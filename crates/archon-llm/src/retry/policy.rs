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

/// The error a stream that failed before any content returns, classed by the
/// provider's error type (Issue 364).
///
/// [`classify`] decides on this value, so the class must match the cause:
/// only a transport fault, a 5xx (`api_error`, `server_error`), an overload
/// (`overloaded_error`, 529) and a rate limit may be retried. A request the
/// provider rejected (invalid request, auth, permission, not found, and any
/// type this list does not know that names no transport fault) fails fast,
/// with the provider's text kept.
pub(super) fn stream_error_before_content(error_type: &str, message: &str) -> LlmError {
    let text = format!("stream failed before producing content ({error_type}): {message}");
    let rejected = |status: u16, message: String| LlmError::Server { status, message };
    match error_type {
        "authentication_error" | "invalid_api_key" => LlmError::Auth(text),
        "permission_error" => rejected(403, text),
        "not_found_error" => rejected(404, text),
        "request_too_large" => rejected(413, text),
        "billing_error" | "insufficient_quota" => LlmError::QuotaExceeded(text),
        "invalid_request_error" => rejected(400, text),
        "api_error" | "server_error" => rejected(500, text),
        "overloaded_error" => rejected(529, text),
        "network"
        | "http_error"
        | "transport_idle"
        | "rate_limit_error"
        | "rate_limit_exceeded" => LlmError::Http(text),
        "protocol" if message.contains("stream ended before message_stop") => LlmError::Http(text),
        _ if stream_error_is_retryable(error_type, message) => LlmError::Http(text),
        _ => rejected(400, text),
    }
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

    /// Fails before the fix: every class came back as `Http`, which reads
    /// as retryable, so a rejected request backed off for a whole window.
    #[test]
    fn a_stream_error_before_content_is_classed_by_its_provider_type() {
        let retry = [
            ("network", "connection reset"),
            ("http_error", "error decoding response body"),
            ("transport_idle", "provider transport stalled"),
            ("protocol", "stream ended before message_stop"),
            ("api_error", "Internal server error"),
            ("server_error", "The server had an error"),
            ("overloaded_error", "Overloaded"),
            (
                "rate_limit_error",
                "Number of requests has exceeded your rate limit",
            ),
            ("rate_limit_exceeded", "Rate limit reached"),
            ("unknown_type", "upstream connect error"),
        ];
        for (error_type, message) in retry {
            let error = stream_error_before_content(error_type, message);
            assert_eq!(
                classify(&error),
                RetryDecision::Retry,
                "{error_type}: {error}"
            );
        }
        let fail_fast = [
            ("invalid_request_error", "messages: field required", "(400)"),
            (
                "authentication_error",
                "invalid x-api-key",
                "authentication error",
            ),
            ("permission_error", "not allowed to use this model", "(403)"),
            ("not_found_error", "model: claude-x", "(404)"),
            ("translator_error", "unexpected event shape", "(400)"),
            // A rejected request whose text names a transport word still fails fast.
            (
                "invalid_request_error",
                "tool call timed out field",
                "(400)",
            ),
        ];
        for (error_type, message, shown) in fail_fast {
            let error = stream_error_before_content(error_type, message);
            assert_eq!(
                classify(&error),
                RetryDecision::FailFast,
                "{error_type}: {error}"
            );
            let text = error.to_string();
            assert!(text.contains(shown) && text.contains(message), "{text}");
            assert!(text.contains(error_type), "{text}");
        }
    }

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
