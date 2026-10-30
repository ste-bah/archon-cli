//! Operational recovery semantics, independent of bounded interactive retries.
use crate::provider::LlmError;

/// Legacy callers use different levels of rate-limit evidence. Keep their
/// vocabulary together without broadening a caller's classification policy.
#[derive(Clone, Copy)]
pub enum RateLimitEvidence {
    Phrase,
    StatusOrPhrase,
    RequestPressure,
    SubagentFailure,
    Codex,
}

/// Match the plain phrase used by provider and workflow text classifiers.
pub fn rate_limit_text_matches(message: &str) -> bool {
    rate_limit_matches(message, RateLimitEvidence::Phrase)
}

/// Shared matcher with each existing caller's specificity preserved.
pub fn rate_limit_matches(message: &str, evidence: RateLimitEvidence) -> bool {
    match evidence {
        RateLimitEvidence::Phrase => message.contains("rate limit"),
        RateLimitEvidence::StatusOrPhrase => {
            message.contains("429") || message.contains("rate limit")
        }
        RateLimitEvidence::RequestPressure => {
            rate_limit_matches(message, RateLimitEvidence::StatusOrPhrase)
                || message.contains("rate_limited")
                || message.contains("rate_limit_error")
        }
        RateLimitEvidence::SubagentFailure => {
            message.contains("429 ")
                || message.contains(" 429")
                || message == "429"
                || message.contains(" rate limit ")
                || message.starts_with("rate limit ")
                || message.ends_with(" rate limit")
                || message.contains("rate-limit")
        }
        RateLimitEvidence::Codex => {
            message.contains("rate limit")
                || message.contains("rate-limit")
                || message.contains("ratelimit")
        }
    }
}

/// The workflow send-message rule after its no-progress guard: a leading
/// status decides, then a context fault refuses, then transient prose
/// retries. One source for the workflow text policy and typed faults.
pub fn send_retry_text(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    if let Some(status) = status_field(&text) {
        return matches!(status, 408 | 429 | 500..=599);
    }
    if workflow_context_fault(&text) {
        return false;
    }
    transient_text_matches(&text)
}

/// The first known status in `msg`: retryable codes first, then the
/// non-retryable ones; (false, 0) when none appears. One source for the
/// pipeline retry scan and typed faults.
pub fn http_status_pattern(msg: &str) -> (bool, u16) {
    // Retryable status codes.
    for code in &[429u16, 500, 502, 503] {
        let pattern = format!("{}", code);
        if msg.contains(&pattern) {
            return (true, *code);
        }
    }

    // Non-retryable status codes.
    for code in &[400u16, 401, 403, 404, 422] {
        let pattern = format!("{}", code);
        if msg.contains(&pattern) {
            return (false, *code);
        }
    }

    // Default: non-retryable.
    (false, 0)
}

/// The pipeline's transient-attempt text rule.
pub fn pipeline_transient_text(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "http error",
        "error decoding response body",
        "connection reset",
        "connection closed",
        "connection refused",
        "timed out",
        "timeout",
        "temporarily unavailable",
        "500",
        "502",
        "503",
        "504",
    ]
    .iter()
    .any(|marker| message.contains(marker))
        || pipeline_overload_code(&message)
}

/// The pipeline's overload error codes, as whole words of a lowercased text.
pub fn pipeline_overload_code(message: &str) -> bool {
    [
        "overloaded_error",
        "server_is_overloaded",
        "rate_limit_error",
    ]
    .iter()
    .any(|code| {
        message.match_indices(code).any(|(index, _)| {
            let before = message[..index].chars().next_back();
            let after = message[index + code.len()..].chars().next();
            before.is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
                && after.is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
        })
    })
}

/// Retry log category of a diagnostic text; one rule for typed and text faults.
pub fn retry_error_class_text(message: &str) -> &'static str {
    let text = message.to_ascii_lowercase();
    if text.contains("rate limit") || text.contains("rate limited") || text.contains("429") {
        "rate_limit"
    } else if text.contains("overloaded") {
        "overload"
    } else if text.contains("connection") || text.contains("socket") || text.contains("request") {
        "connection"
    } else {
        "transient"
    }
}

/// Permanent text guards shared by raw and captured workflow classification.
pub fn workflow_context_fault(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    [
        "context window exceeded",
        "context-too-large",
        "context too large",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

/// Fanout historically matches body fragments without status precedence.
pub fn fanout_retry_text_matches(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    rate_limit_matches(&text, RateLimitEvidence::StatusOrPhrase)
        || [
            "timeout",
            "temporar",
            "error decoding response body",
            "error sending request",
            "request failed",
            "connection reset",
            "connection closed",
            "broken pipe",
            "502",
            "503",
            "504",
        ]
        .iter()
        .any(|marker| text.contains(marker))
}

/// Construct a stored diagnostic with the same redactor used for stream faults.
/// Sanitize before storage so both error fields and human output are safe.
pub(crate) fn diagnostic_message(message: &str) -> String {
    archon_observability::redaction::redact_text(message)
}

/// Append a supplied diagnostic without changing message-less fault display.
pub(crate) fn diagnostic_suffix(message: &str) -> String {
    if message.is_empty() {
        String::new()
    } else {
        format!(": {message}")
    }
}

impl LlmError {
    /// A typed rejection wins over diagnostic text quoted by the provider.
    pub fn is_operational(&self) -> bool {
        match self {
            Self::RateLimited { .. } | Self::Overloaded { .. } | Self::Transport(_) => true,
            Self::Server { status, .. } => matches!(*status, 408 | 429 | 500..=599),
            Self::Http(text) => http_operational(text.raw()),
            _ => false,
        }
    }

    /// A known request rejection stays permanent even when its body quotes
    /// a timeout. Legacy HTTP diagnostics retain a leading status field.
    pub fn is_permanent(&self) -> bool {
        match self {
            Self::Http(text) => status_field(text.raw())
                .is_some_and(|status| (400..500).contains(&status) && !matches!(status, 408 | 429)),
            _ => !self.is_operational(),
        }
    }

    /// Provider metadata survives display text and context wrappers.
    pub fn retry_after_hint(&self) -> Option<u64> {
        match self {
            Self::RateLimited {
                retry_after_secs,
                pipeline_retry_after_secs,
                from_provider,
                ..
            } => pipeline_retry_after_secs.or_else(|| from_provider.then_some(*retry_after_secs)),
            Self::Overloaded {
                retry_after_secs, ..
            } => *retry_after_secs,
            Self::Server {
                pipeline_retry_after_secs,
                ..
            } => *pipeline_retry_after_secs,
            _ => None,
        }
    }
}

pub(crate) fn http_operational(text: &str) -> bool {
    http_operational_classification(text).unwrap_or(false)
}

// None means no signal; Some(false) is an explicit exclusion or rejection.
pub(crate) fn http_operational_classification(text: &str) -> Option<bool> {
    http_operational_parts(
        status_field(text),
        http_leading_exclusion(text),
        http_exclusion_matches(text),
        http_marker_matches(text),
    )
}

/// The original precedence: leading status, then exclusions, then markers.
pub(crate) fn http_operational_parts(
    status: Option<u16>,
    leading_exclusion: bool,
    exclusion: bool,
    marker: bool,
) -> Option<bool> {
    if let Some(status) = status {
        return Some(matches!(status, 408 | 429 | 500..=599));
    }
    // An opened stream's idle/protocol exhaustion is no-progress,
    // not proof of an unavailable endpoint. It earns no clock credit.
    if leading_exclusion || exclusion {
        return Some(false);
    }
    marker.then_some(true)
}

/// Position-dependent exclusion: only the start of the whole text counts.
pub(crate) fn http_leading_exclusion(text: &str) -> bool {
    text.starts_with("protocol:") || text.starts_with("parse_error:")
}

/// Substring exclusions: true for a composed text when true for any part.
pub(crate) fn http_exclusion_matches(text: &str) -> bool {
    text.contains(crate::transport_idle::TRANSPORT_STALL_MARKER)
        || text.contains("provider transport stalled:")
        || text.contains("stream idle timeout")
        || text.contains("no complete answer")
}

pub(crate) fn http_marker_matches(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    rate_limit_text_matches(&text)
        || [
            "connection",
            "socket",
            "broken pipe",
            "error sending request",
            "request failed",
            "http_error",
            "overloaded",
            "server_error",
            "timed out",
            "timeout",
            "temporar",
        ]
        .iter()
        .any(|marker| text.contains(marker))
}

/// Legacy transient signals, independent of operational clock-credit policy.
pub fn transient_text_matches(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    rate_limit_text_matches(&text)
        || [
            "llm stream error",
            "server_error",
            "error decoding response body",
            "error sending request",
            "request failed",
            "connection reset",
            "connection closed",
            "connection refused",
            "broken pipe",
            "timed out",
            "timeout",
            "temporar",
            "server overloaded",
        ]
        .iter()
        .any(|marker| text.contains(marker))
}

/// Parse only a leading status field, never codes quoted in a response body.
pub fn status_field(text: &str) -> Option<u16> {
    let text = text.trim().to_ascii_lowercase();
    let value = [
        "http error: ",
        "http ",
        "status ",
        "server error (",
        "server error ",
    ]
    .iter()
    .find_map(|prefix| text.strip_prefix(prefix))
    .unwrap_or(&text);
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    (digits.len() == 3).then(|| digits.parse().ok()).flatten()
}

// A workflow caller owns the durable retry and its host clocks. Provider-local
// interactive retry windows must return the original fault to that caller.
type Response = Option<(u16, Option<u64>)>;
tokio::task_local! { static DEFER_RETRY: tokio::sync::watch::Sender<Response>; }
pub fn retry_is_deferred() -> bool {
    DEFER_RETRY.try_with(|_| ()).is_ok()
}
pub async fn defer_retry<T>(work: impl std::future::Future<Output = T>) -> T {
    DEFER_RETRY
        .scope(tokio::sync::watch::channel(None).0, work)
        .await
}

/// Begin a transport attempt without retaining an earlier response's metadata.
pub fn begin_request() {
    let _ = DEFER_RETRY.try_with(|response| response.send_replace(None));
}

/// Keep received headers available even if reading the body never completes.
pub fn note_response(status: u16, retry_after: Option<u64>) {
    let _ = DEFER_RETRY.try_with(|response| response.send_replace(Some((status, retry_after))));
}

pub fn pending_response() -> Response {
    DEFER_RETRY
        .try_with(|response| *response.borrow())
        .ok()
        .flatten()
}

pub fn response_watch() -> Option<tokio::sync::watch::Receiver<Response>> {
    DEFER_RETRY.try_with(|response| response.subscribe()).ok()
}

/// Preserve an actual response status when its body stalls; absence of headers
/// means the connection never answered, rather than an opened stream stalling.
pub fn opening_timeout() -> LlmError {
    match pending_response() {
        Some((status, hint)) if status >= 400 => LlmError::Server {
            status,
            message: "error response body timed out".into(),
            pipeline_retry_after_secs: hint,
        },
        _ => LlmError::Transport("connection timed out before response headers".into()),
    }
}

#[path = "provider_fault_transport.rs"]
mod transport;

pub(crate) use transport::{
    api_body_read_error, api_open_transport, body_read_error, native_transport_event,
    structured_rejection, transport_event, unknown,
};
pub use transport::{
    error_event, from_native_transport, from_status, from_stream_error, from_transport,
};

#[cfg(test)]
#[path = "provider_fault_characterization_tests.rs"]
mod c0_characterization;
