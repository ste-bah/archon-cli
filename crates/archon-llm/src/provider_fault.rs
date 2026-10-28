//! Operational recovery semantics, independent of bounded interactive retries.
use crate::provider::LlmError;

impl LlmError {
    /// A typed rejection wins over diagnostic text quoted by the provider.
    pub fn is_operational(&self) -> bool {
        match self {
            Self::RateLimited { .. } | Self::Overloaded { .. } | Self::Transport(_) => true,
            Self::Server { status, .. } => matches!(*status, 408 | 429 | 500..=599),
            Self::Http(text) => {
                if let Some(status) = status_field(text) {
                    return matches!(status, 408 | 429 | 500..=599);
                }
                // An opened stream's idle/protocol exhaustion is no-progress,
                // not proof of an unavailable endpoint. It earns no clock credit.
                if text.starts_with("protocol:")
                    || text.starts_with("parse_error:")
                    || text.contains(crate::transport_idle::TRANSPORT_STALL_MARKER)
                    || text.contains("provider transport stalled:")
                    || text.contains("stream idle timeout")
                    || text.contains("no complete answer")
                {
                    return false;
                }
                let text = text.to_ascii_lowercase();
                [
                    "connection",
                    "socket",
                    "broken pipe",
                    "error sending request",
                    "request failed",
                    "http_error",
                    "overloaded",
                    "server_error",
                    "rate limit",
                    "timed out",
                    "timeout",
                    "temporar",
                ]
                .iter()
                .any(|marker| text.contains(marker))
            }
            _ => false,
        }
    }

    /// A known request rejection stays permanent even when its body quotes
    /// a timeout. Legacy HTTP diagnostics retain a leading status field.
    pub fn is_permanent(&self) -> bool {
        match self {
            Self::Http(text) => status_field(text)
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
        error_type: kind.to_string(),
        message: archon_observability::redaction::redact_text(body),
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
            return LlmError::Serialize(error.to_string());
        }
        source = cause.source();
    }
    if error.is_builder() || error.is_redirect() {
        LlmError::Serialize(error.to_string())
    } else if let Some(status) = error.status() {
        from_status(status.as_u16(), &Default::default(), error.to_string())
    } else {
        from_native_transport(error)
    }
}

/// For adapters that expose a native transport failure without an HTTP error.
/// Call only at a transport boundary, never with an arbitrary diagnostic.
pub fn from_native_transport(error: impl std::fmt::Display) -> LlmError {
    LlmError::Transport(error.to_string())
}

/// Unknown causes retain their diagnostic without inventing an outage class.
/// An incomplete/idle stream does not prove the endpoint was unavailable.
pub(crate) fn unknown(message: impl Into<String>) -> LlmError {
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
        401 | 403 => LlmError::Auth(body),
        429 => LlmError::RateLimited {
            retry_after_secs: 0,
            from_provider: false,
            pipeline_retry_after_secs: hint,
        },
        _ => LlmError::Server {
            status,
            message: body,
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
pub fn from_stream_error(kind: &str, message: &str) -> LlmError {
    let message = archon_observability::redaction::redact_text(message);
    match kind {
        "aborted" => LlmError::Aborted,
        "network" | "http_error" | "transport" | "connection_error" | "timeout_error" => {
            from_native_transport(message)
        }
        "rate_limited" | "rate_limit_error" | "rate_limit_exceeded" => LlmError::RateLimited {
            retry_after_secs: 0,
            from_provider: false,
            pipeline_retry_after_secs: None,
        },
        "overloaded_error" | "server_overloaded" | "server_error" | "api_error" => {
            LlmError::Overloaded {
                status: None,
                retry_after_secs: None,
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
        _ => unknown(format!("{kind}: {message}")),
    }
}

pub(crate) fn transport_event(error: reqwest::Error) -> crate::streaming::StreamEvent {
    error_event(from_transport(error))
}

pub(crate) fn error_event(error: LlmError) -> crate::streaming::StreamEvent {
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
    let kind = kind.to_string();
    let message = match error {
        LlmError::Transport(message)
        | LlmError::Serialize(message)
        | LlmError::Http(message)
        | LlmError::Rejected { message, .. } => message,
        error => error.to_string(),
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
        } => ApiError::RateLimited {
            retry_after_secs: pipeline_retry_after_secs.unwrap_or(retry_after_secs),
            from_provider: from_provider || pipeline_retry_after_secs.is_some(),
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
            message: other.to_string(),
        },
    }
}

pub(crate) fn native_transport_event(
    error: impl std::fmt::Display,
) -> crate::streaming::StreamEvent {
    error_event(from_native_transport(error))
}
