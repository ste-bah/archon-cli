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

/// Structured request rejections take precedence over an intermediary status.
/// Keep the original payload so the rejected field and its remedy survive.
pub(crate) fn structured_rejection(status: u16, body: &str) -> Option<LlmError> {
    if !matches!(status, 408 | 429 | 500..=599) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let error = value.get("error")?;
    let kind = ["code", "type"]
        .iter()
        .filter_map(|field| error.get(field)?.as_str())
        .find(|kind| {
            matches!(
                *kind,
                "insufficient_quota"
                    | "billing_hard_limit_reached"
                    | "billing_error"
                    | "model_not_found"
                    | "invalid_model"
                    | "invalid_request_error"
                    | "authentication_error"
                    | "invalid_api_key"
                    | "permission_error"
                    | "not_found_error"
                    | "request_too_large"
            )
        })?;
    Some(LlmError::Rejected {
        error_type: kind.to_string().into(),
        // 86fbab995 redacted the body here, before any classifier read it.
        message: crate::provider::DiagnosticMessage::redacted_at_source(body),
    })
}

/// Convert the native HTTP cause before its display text loses classification.
/// Request construction and JSON syntax/data errors require a changed request;
/// connect, DNS, TLS, timeout, reset and receive failures require recovery.
pub fn from_transport(error: reqwest::Error) -> LlmError {
    use std::error::Error;
    let mut source = error.source();
    while let Some(cause) = source {
        if cause.downcast_ref::<serde_json::Error>().is_some() {
            return LlmError::Serialize(error.to_string().into());
        }
        source = cause.source();
    }
    if error.is_builder() || error.is_redirect() {
        LlmError::Serialize(error.to_string().into())
    } else if let Some(status) = error.status() {
        from_status(status.as_u16(), &Default::default(), error.to_string())
    } else {
        from_native_transport(error)
    }
}

/// For adapters that expose a native transport failure without an HTTP error.
/// Call only at a transport boundary, never with an arbitrary diagnostic.
pub fn from_native_transport(error: impl std::fmt::Display) -> LlmError {
    LlmError::Transport(error.to_string().into())
}

/// Unknown causes retain their diagnostic without inventing an outage class.
/// An incomplete/idle stream does not prove the endpoint was unavailable.
pub(crate) fn unknown(message: impl Into<crate::provider::DiagnosticMessage>) -> LlmError {
    LlmError::Http(message.into())
}

/// Interpret a real response, retaining status precedence and header metadata.
/// Structured request rejection overrides a proxy's otherwise transient status.
pub fn from_status(status: u16, headers: &reqwest::header::HeaderMap, body: String) -> LlmError {
    if let Some(error) = structured_rejection(status, &body) {
        return error;
    }
    let hint = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            crate::anthropic_support::retry_after_from_header(value, chrono::Utc::now())
        });
    match status {
        401 | 403 => LlmError::Auth(body.into()),
        429 => LlmError::RateLimited {
            retry_after_secs: 0,
            from_provider: false,
            pipeline_retry_after_secs: hint,
            message: crate::provider::DiagnosticMessage::redacted_at_source(&body),
        },
        _ => LlmError::Server {
            status,
            message: body.into(),
            pipeline_retry_after_secs: hint,
        },
    }
}

/// A failed error-body read keeps its received status; a successful response's
/// interrupted body is a transport fault. Never infer status from body text.
pub(crate) fn body_read_error(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    error: reqwest::Error,
) -> LlmError {
    if status >= 400 {
        from_status(
            status,
            headers,
            format!("error response body failed: {error}"),
        )
    } else {
        from_transport(error)
    }
}

/// Known transient event classes remain typed across executor outcomes. The
/// interactive stream decorator retains its existing pre-content diagnostics.
/// Preserve the producer's event class through completion and turn adapters.
pub fn from_stream_error(
    kind: &str,
    message: impl Into<crate::provider::DiagnosticMessage>,
) -> LlmError {
    // 86fbab995 redacted the event text first; every mapping below saw that.
    let message = crate::provider::DiagnosticMessage::redacted_at_source(message.into().as_str());
    match kind {
        "aborted" => LlmError::Aborted,
        "network" | "http_error" | "transport" | "connection_error" | "timeout_error" => {
            LlmError::Transport(message)
        }
        "rate_limited" | "rate_limit_error" | "rate_limit_exceeded" => LlmError::RateLimited {
            retry_after_secs: 0,
            from_provider: false,
            pipeline_retry_after_secs: None,
            message,
        },
        "overloaded_error" | "server_overloaded" | "server_error" | "api_error" => {
            LlmError::Overloaded {
                status: None,
                retry_after_secs: None,
                message,
            }
        }
        "billing_error" | "insufficient_quota" | "billing_hard_limit_reached" => {
            LlmError::QuotaExceeded(message)
        }
        "authentication_error"
        | "invalid_api_key"
        | "invalid_request_error"
        | "permission_error"
        | "not_found_error"
        | "request_too_large"
        | "model_not_found"
        | "invalid_model" => LlmError::Rejected {
            error_type: kind.into(),
            message,
        },
        _ => unknown(message.with_prefix(&format!("{kind}: "))),
    }
}

pub(crate) fn transport_event(error: reqwest::Error) -> crate::streaming::StreamEvent {
    error_event(from_transport(error))
}

pub fn error_event(error: LlmError) -> crate::streaming::StreamEvent {
    let kind = match &error {
        LlmError::Transport(_) => "network",
        LlmError::Serialize(_) | LlmError::Unsupported(_) | LlmError::ProviderNotFound { .. } => {
            "invalid_request_error"
        }
        LlmError::RateLimited { .. } => "rate_limit_error",
        LlmError::Overloaded { .. } => "overloaded_error",
        LlmError::Server { .. } if error.is_operational() => "server_error",
        LlmError::Server { .. } => "invalid_request_error",
        LlmError::Auth(_) => "authentication_error",
        LlmError::QuotaExceeded(_) => "insufficient_quota",
        LlmError::ContextWindowExceeded { .. } => "request_too_large",
        LlmError::Rejected { error_type, .. } => error_type,
        LlmError::Aborted => "aborted",
        LlmError::Http(_) => "protocol",
    };
    let kind = if let LlmError::Rejected { error_type, .. } = &error {
        error_type.clone()
    } else {
        kind.into()
    };
    let message = match &error {
        LlmError::Transport(message)
        | LlmError::Serialize(message)
        | LlmError::Http(message)
        | LlmError::Rejected { message, .. } => message.clone(),
        LlmError::RateLimited { message, .. } | LlmError::Overloaded { message, .. }
            if !message.is_empty() =>
        {
            message.clone()
        }
        // 86fbab995 used the Display text for every other variant.
        error => error.display_message(),
    };
    crate::streaming::StreamEvent::Error {
        error_type: kind,
        message,
    }
}

/// Preserve the opening read-idle signal before converting the native cause.
/// The provider boundary decides whether the caller defers outage recovery.
pub(crate) fn api_open_transport(error: reqwest::Error) -> crate::anthropic::ApiError {
    if error.is_timeout() {
        crate::anthropic::ApiError::IdleTimeout
    } else {
        api_transport(error)
    }
}

pub(crate) fn api_transport(error: reqwest::Error) -> crate::anthropic::ApiError {
    into_api(from_transport(error))
}

pub(crate) fn api_body_read_error(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    error: reqwest::Error,
) -> crate::anthropic::ApiError {
    into_api(body_read_error(status, headers, error))
}

fn into_api(error: LlmError) -> crate::anthropic::ApiError {
    use crate::anthropic::ApiError;
    match error {
        LlmError::Transport(message) => ApiError::Transport(message),
        LlmError::Serialize(message) => ApiError::SerializeError(message),
        LlmError::Server {
            status,
            message,
            pipeline_retry_after_secs,
        } => ApiError::ServerError {
            status,
            message,
            retry_after_secs: pipeline_retry_after_secs,
        },
        LlmError::Auth(message) => ApiError::AuthError(message),
        LlmError::RateLimited {
            retry_after_secs,
            from_provider,
            pipeline_retry_after_secs,
            message,
        } => ApiError::RateLimited {
            retry_after_secs: pipeline_retry_after_secs.unwrap_or(retry_after_secs),
            from_provider: from_provider || pipeline_retry_after_secs.is_some(),
            message: crate::provider::DiagnosticMessage::redacted_at_source(message.as_str()),
        },
        LlmError::Rejected {
            error_type,
            message,
        } => ApiError::Rejected {
            error_type,
            message,
        },
        other => ApiError::Rejected {
            error_type: "request_error".into(),
            message: other.display_message(),
        },
    }
}

pub(crate) fn native_transport_event(
    error: impl std::fmt::Display,
) -> crate::streaming::StreamEvent {
    error_event(from_native_transport(error))
}

#[cfg(test)]
mod diagnostic_redaction_tests {
    fn convert(status: u16, body: &str) -> crate::provider::LlmError {
        super::from_status(status, &Default::default(), body.into())
    }
    include!("provider_fault_redaction_cases.rs");
}

#[cfg(test)]
#[test]
fn server_redaction_provider_fault_forwarder() {
    crate::server_redaction_tests::check(into_api(LlmError::Server {
        status: 500,
        message: crate::server_redaction_tests::BODY.into(),
        pipeline_retry_after_secs: Some(7),
    }));
}

#[cfg(test)]
#[path = "provider_fault_characterization_tests.rs"]
mod c0_characterization;
