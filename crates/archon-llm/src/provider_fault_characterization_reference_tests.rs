#![allow(dead_code)]
use crate::context_window::RequestPressureKind;
#[allow(dead_code)]
pub enum LlmError {
    Http(String),
    Transport(String),
    RateLimited {
        retry_after_secs: u64,
        from_provider: bool,
        pipeline_retry_after_secs: Option<u64>,
        message: String,
    },
    Overloaded {
        status: Option<u16>,
        retry_after_secs: Option<u64>,
        message: String,
    },
    QuotaExceeded(String),
    Aborted,
    Server {
        status: u16,
        message: String,
    },
    Rejected {
        error_type: String,
        message: String,
    },
    Other,
    ContextWindowExceeded {
        provider_message: String,
        provider: Option<String>,
        model: Option<String>,
    },
}

// Frozen 86fbab995 constant, so the baseline never reads current code.
const TRANSPORT_STALL_MARKER: &str =
    "provider transport stall (no answer for a whole no-progress window; resumable):";
pub(super) fn status_field(text: &str) -> Option<u16> {
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
impl LlmError {
    // Freeze baseline bodies: rustfmt otherwise rewrites match-arm blocks.
        #[rustfmt::skip]
    pub(super) fn request_pressure_kind(&self) -> Option<crate::context_window::RequestPressureKind> {
            match self {
                Self::ContextWindowExceeded { .. } => {
                    Some(crate::context_window::RequestPressureKind::AggregateContext)
                }
                Self::Http(message) => {
                    frozen::context_window::classify_request_pressure_error(None, None, None, message)
                }
                Self::Server {
                    status, message, ..
                } => frozen::context_window::classify_request_pressure_error(
                    Some(*status),
                    None,
                    None,
                    message,
                ),
                Self::Rejected {
                    error_type,
                    message,
                } => frozen::context_window::classify_request_pressure_error(
                    None,
                    Some(error_type),
                    None,
                    message,
                ),
                _ => None,
            }
        }

    // Freeze baseline bodies: rustfmt otherwise rewrites match-arm blocks.
        #[rustfmt::skip]
    pub(super) fn is_context_window_exceeded(&self) -> bool {
            match self {
                Self::ContextWindowExceeded { .. } => true,
                Self::Http(message) => frozen::context_window::classify_context_window_error(
                    None, None, None, message, None, None,
                )
                .is_some(),
                Self::Server {
                    status, message, ..
                } => frozen::context_window::classify_context_window_error(
                    Some(*status),
                    None,
                    None,
                    message,
                    None,
                    None,
                )
                .is_some(),
                Self::Rejected {
                    error_type,
                    message,
                } => frozen::context_window::classify_context_window_error(
                    None,
                    Some(error_type),
                    None,
                    message,
                    None,
                    None,
                )
                .is_some(),
                _ => false,
            }
        }

    pub(super) fn is_operational(&self) -> bool {
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
                    || text.contains(TRANSPORT_STALL_MARKER)
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
}
// Paths inside the frozen bodies name the frozen copies in this module.
mod frozen {
    pub(super) mod context_window {
        pub(in super::super) use super::super::{
            classify_context_window_error, classify_request_pressure_error,
        };
    }
}
impl LlmError {
    pub(super) fn is_permanent(&self) -> bool {
        match self {
            Self::Http(text) => status_field(text)
                .is_some_and(|status| (400..500).contains(&status) && !matches!(status, 408 | 429)),
            _ => !self.is_operational(),
        }
    }
    // 86fbab995's Display templates, over the raw diagnostic text.
    pub(super) fn display(&self) -> String {
        let suffix = |message: &str| {
            if message.is_empty() {
                String::new()
            } else {
                format!(": {message}")
            }
        };
        match self {
            Self::Http(text) | Self::Transport(text) => format!("HTTP error: {text}"),
            Self::RateLimited {
                retry_after_secs,
                message,
                ..
            } => format!(
                "rate limited: retry after {retry_after_secs}s{}",
                suffix(message)
            ),
            Self::Overloaded { message, .. } => format!("server overloaded{}", suffix(message)),
            Self::QuotaExceeded(text) => format!("quota exceeded: {text}"),
            Self::Aborted => "request aborted".into(),
            Self::Server { status, message } => format!("server error ({status}): {message}"),
            Self::Rejected {
                error_type,
                message,
            } => format!("provider rejected the request ({error_type}): {message}"),
            Self::Other => String::new(),
            Self::ContextWindowExceeded {
                provider_message, ..
            } => format!("context window exceeded: {provider_message}"),
        }
    }
    pub(super) fn variant(&self) -> &'static str {
        match self {
            Self::Http(_) => "http",
            Self::Transport(_) => "transport",
            Self::RateLimited { .. } => "rate_limited",
            Self::Overloaded { .. } => "overloaded",
            Self::QuotaExceeded(_) => "quota",
            Self::Aborted => "aborted",
            Self::Server { .. } => "server",
            Self::Rejected { .. } => "rejected",
            Self::Other => "other",
            Self::ContextWindowExceeded { .. } => "context",
        }
    }
}
// 86fbab995's diagnostic_message: its redactor, pinned below by
// c0_redactor_is_frozen_86fbab995.
fn diagnostic_message(message: &str) -> String {
    archon_observability::redaction::redact_text(message)
}
pub fn from_native_transport(error: impl std::fmt::Display) -> LlmError {
    LlmError::Transport(error.to_string())
}
pub(crate) fn unknown(message: impl Into<String>) -> LlmError {
    LlmError::Http(message.into())
}
pub fn from_stream_error(kind: &str, message: &str) -> LlmError {
    let message = diagnostic_message(message);
    match kind {
        "aborted" => LlmError::Aborted,
        "network" | "http_error" | "transport" | "connection_error" | "timeout_error" => {
            from_native_transport(message)
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
        _ => unknown(format!("{kind}: {message}")),
    }
}
pub(super) fn classify_context_window_error(
    status: Option<u16>,
    error_type: Option<&str>,
    code: Option<&str>,
    message: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> Option<LlmError> {
    let mut haystack = String::new();
    if let Some(error_type) = error_type {
        haystack.push_str(error_type);
        haystack.push(' ');
    }
    if let Some(code) = code {
        haystack.push_str(code);
        haystack.push(' ');
    }
    haystack.push_str(message);
    let lower = haystack.to_ascii_lowercase();

    let status_matches = status.is_none_or(|s| matches!(s, 400 | 413 | 422));
    let text_matches = [
        "context_length_exceeded",
        "context window",
        "context length",
        "maximum context",
        "max context",
        "prompt is too long",
        "prompt too long",
        "too many tokens",
        "too many input tokens",
        "token limit",
        "input is too long",
        "request too large",
    ]
    .iter()
    .any(|needle| lower.contains(needle));

    if status_matches && text_matches {
        Some(LlmError::ContextWindowExceeded {
            provider_message: message.to_string(),
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
        })
    } else {
        None
    }
}
pub(super) fn classify_request_pressure_error(
    status: Option<u16>,
    error_type: Option<&str>,
    code: Option<&str>,
    message: &str,
) -> Option<RequestPressureKind> {
    let status_matches = status.is_none_or(|value| matches!(value, 400 | 413 | 422));
    if !status_matches {
        return None;
    }
    let mut text = String::new();
    if let Some(error_type) = error_type {
        text.push_str(error_type);
        text.push(' ');
    }
    if let Some(code) = code {
        text.push_str(code);
        text.push(' ');
    }
    text.push_str(message);
    let lower = text.to_ascii_lowercase();
    let field_limit = [
        "string should have at most",
        "string too long",
        "maximum length",
        "max string length",
        "field too long",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if field_limit
        && ["tool_result", "tool result", ".output", ".content"]
            .iter()
            .any(|needle| lower.contains(needle))
    {
        return Some(RequestPressureKind::ToolResultField);
    }
    classify_context_window_error(status, error_type, code, message, None, None)
        .map(|_| RequestPressureKind::AggregateContext)
}
// src/runtime/provider_limit_windows.rs at 86fbab995 (reset hints).
use chrono::{DateTime, Duration, Utc};
pub(super) fn reset_hint_from_text(
    message: &str,
    observed_at: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    rfc3339_hint(message).or_else(|| relative_duration_hint(message, observed_at))
}

fn rfc3339_hint(message: &str) -> Option<DateTime<Utc>> {
    message
        .split_ascii_whitespace()
        .map(|token| {
            token
                .trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']'))
        })
        .find_map(|token| {
            DateTime::parse_from_rfc3339(token)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
}

fn relative_duration_hint(message: &str, observed_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let tokens: Vec<String> = message
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    for pair in tokens.windows(2) {
        let Ok(amount) = pair[0].parse::<i64>() else {
            continue;
        };
        let Some(seconds) = unit_seconds(&pair[1]).map(|unit| amount.saturating_mul(unit)) else {
            continue;
        };
        if seconds > 0 {
            return Some(observed_at + Duration::seconds(seconds));
        }
    }
    None
}

fn unit_seconds(unit: &str) -> Option<i64> {
    match unit {
        "second" | "seconds" | "sec" | "secs" => Some(1),
        "minute" | "minutes" | "min" | "mins" => Some(60),
        "hour" | "hours" | "hr" | "hrs" => Some(60 * 60),
        "day" | "days" => Some(24 * 60 * 60),
        _ => None,
    }
}
// crates/archon-llm/src/retry/policy.rs at 86fbab995, redaction omitted.
pub(super) fn stream_error_before_content(error_type: &str, message: &str) -> LlmError {
    let message = diagnostic_message(message);
    let diagnostic = format!("stream failed before producing content ({error_type}): {message}");
    let error = from_stream_error(error_type, &diagnostic);
    if error.is_operational() {
        from_native_transport(diagnostic)
    } else {
        error
    }
}
pub(super) fn is_cancelled_stream_error(error_type: &str, message: &str) -> bool {
    let error_type = error_type.trim().to_ascii_lowercase();
    if matches!(
        error_type.as_str(),
        "cancelled"
            | "canceled"
            | "user_cancelled"
            | "user_canceled"
            | "client_cancelled"
            | "client_canceled"
            | "operation_cancelled"
            | "operation_canceled"
            | "request_cancelled"
            | "request_canceled"
    ) {
        return true;
    }
    let message = message.trim().to_ascii_lowercase();
    message.contains("cancelled by user")
        || message.contains("canceled by user")
        || message.contains("user cancelled")
        || message.contains("user canceled")
        || message.contains("aborted by user")
        || message.contains("user aborted")
}
