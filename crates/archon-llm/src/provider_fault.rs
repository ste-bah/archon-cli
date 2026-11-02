//! Operational recovery semantics, independent of bounded interactive retries.
use crate::provider::LlmError;

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

#[path = "provider_fault_text.rs"]
mod text;

use text::http_operational;
pub use text::{
    RateLimitEvidence, fanout_retry_text_matches, http_status_pattern, pipeline_overload_code,
    pipeline_transient_text, rate_limit_matches, rate_limit_text_matches, retry_error_class_text,
    send_retry_text, status_field, transient_text_matches, workflow_context_fault,
};
