// C0: reference function bodies copied unchanged from 86fbab995.
// Intended classification differences: none. Legacy-only types keep raw test inputs.
// Only reference visibility is widened to pub(super); function bodies are unchanged.
use super::*;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/characterization/classifier_characterization_rows.rs"
));
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/characterization/classifier_characterization_variants.rs"
));
mod main_reference {
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
                Self::Http(text) => status_field(text).is_some_and(|status| {
                    (400..500).contains(&status) && !matches!(status, 408 | 429)
                }),
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
                token.trim_matches(|c: char| {
                    matches!(c, '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']')
                })
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
            let Some(seconds) = unit_seconds(&pair[1]).map(|unit| amount.saturating_mul(unit))
            else {
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
        let diagnostic =
            format!("stream failed before producing content ({error_type}): {message}");
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
}
// Adapt legacy test-only types; every reference function body stays unchanged.
fn check_pressure_variant(
    failures: &mut Vec<String>,
    row: usize,
    name: &str,
    fault: &LlmError,
    raw: &str,
) {
    use main_reference::LlmError as MainError;
    let reference = match fault {
        LlmError::Http(_) => MainError::Http(raw.into()),
        // The server message was a redacted DiagnosticMessage at 86fbab995.
        LlmError::Server { status, .. } => MainError::Server {
            status: *status,
            message: archon_observability::redaction::redact_text(raw),
        },
        LlmError::Rejected { error_type, .. } => MainError::Rejected {
            error_type: error_type.to_string(),
            message: raw.into(),
        },
        LlmError::ContextWindowExceeded { .. } => MainError::ContextWindowExceeded {
            provider_message: raw.into(),
            provider: None,
            model: None,
        },
        // Main's getters use their wildcard arm for all remaining families.
        _ => MainError::Other,
    };
    check_row(
        failures,
        row,
        &format!("pressure/{name}"),
        fault.request_pressure_kind(),
        reference.request_pressure_kind(),
    );
    check_row(
        failures,
        row,
        &format!("context/{name}"),
        fault.is_context_window_exceeded(),
        reference.is_context_window_exceeded(),
    );
}
fn variant(fault: &LlmError) -> &'static str {
    match fault {
        LlmError::Http(_) => "http",
        LlmError::Transport(_) => "transport",
        LlmError::RateLimited { .. } => "rate_limited",
        LlmError::Overloaded { .. } => "overloaded",
        LlmError::QuotaExceeded(_) => "quota",
        LlmError::Aborted => "aborted",
        LlmError::Server { .. } => "server",
        LlmError::Rejected { .. } => "rejected",
        LlmError::ContextWindowExceeded { .. } => "context",
        _ => "other",
    }
}
fn check_stream(
    failures: &mut Vec<String>,
    row: usize,
    name: &str,
    fault: LlmError,
    reference: main_reference::LlmError,
) {
    check_row(
        failures,
        row,
        &format!("{name}/variant"),
        variant(&fault),
        reference.variant(),
    );
    check_row(
        failures,
        row,
        &format!("{name}/operational"),
        fault.is_operational(),
        reference.is_operational(),
    );
    // Status evidence in the display text, as pipeline retry scans it.
    for code in [400u16, 401, 403, 404, 422, 429, 500, 502, 503] {
        check_row(
            failures,
            row,
            &format!("{name}/mentions_{code}"),
            fault.mentions_status(code),
            reference.display().contains(&code.to_string()),
        );
    }
    check_row(
        failures,
        row,
        &format!("{name}/permanent"),
        fault.is_permanent(),
        reference.is_permanent(),
    );
    check_row(
        failures,
        row,
        &format!("{name}/pressure"),
        fault.request_pressure_kind(),
        reference.request_pressure_kind(),
    );
    check_row(
        failures,
        row,
        &format!("{name}/context"),
        fault.is_context_window_exceeded(),
        reference.is_context_window_exceeded(),
    );
}
// Stream kinds: main composed "{kind}: {message}" for unknown kinds before classifying.
const STREAM_KINDS: [&str; 15] = [
    "protocol",
    "parse_error",
    "peer",
    "context",
    "HTTP 503",
    "503",
    "",
    "transport_idle",
    "aborted",
    "network",
    "rate_limited",
    "overloaded_error",
    "billing_error",
    "invalid_request_error",
    "request_too_large",
];
// Prefixes the production code composes, plus a split duration.
const RESET_PREFIXES: [&str; 4] = ["", "peer: ", "retry in 5 ", "server error (429): "];
#[test]
fn c0_reset_hint_main_equivalence() {
    let observed = chrono::DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        for prefix in RESET_PREFIXES {
            check_row(
                &mut failures,
                row,
                &format!("reset[{prefix}]"),
                crate::provider::DiagnosticMessage::from(raw.as_str())
                    .with_prefix(prefix)
                    .reset_hint_at(observed),
                main_reference::reset_hint_from_text(&format!("{prefix}{raw}"), observed),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn c0_stream_composition_main_equivalence() {
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        for kind in STREAM_KINDS {
            check_stream(
                &mut failures,
                row,
                &format!("stream[{kind}]"),
                from_stream_error(kind, raw.as_str()),
                main_reference::from_stream_error(kind, raw),
            );
            // The pre-content probe composes a second prefix before the same mapping.
            // The production pre-content forwarder, against its frozen copy.
            check_stream(
                &mut failures,
                row,
                &format!("pre_content[{kind}]"),
                crate::retry::stream_error_before_content(kind, raw.as_str()),
                main_reference::stream_error_before_content(kind, raw),
            );
            let prefix = format!("stream failed before producing content ({kind}): ");
            check_stream(
                &mut failures,
                row,
                &format!("nested[{kind}]"),
                from_stream_error(
                    kind,
                    crate::provider::DiagnosticMessage::from(raw.as_str()).with_prefix(&prefix),
                ),
                main_reference::from_stream_error(kind, &format!("{prefix}{raw}")),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn c0_provider_fault_main_equivalence() {
    let mut failures = Vec::new();
    for (row, raw) in characterization_rows().iter().enumerate() {
        for (name, fault, _) in characterization_variants(raw) {
            check_pressure_variant(&mut failures, row, name, &fault, raw);
        }
        // Also exercise pressure assembled from the structured kind and body.
        let contextual = LlmError::Rejected {
            error_type: "context_length_exceeded".into(),
            message: raw.into(),
        };
        check_pressure_variant(
            &mut failures,
            row,
            "rejected_context_kind",
            &contextual,
            raw,
        );
        let diagnostic = crate::provider::DiagnosticMessage::from(raw);
        check_row(
            &mut failures,
            row,
            "cancel",
            diagnostic.is_user_cancelled(),
            main_reference::is_cancelled_stream_error("", raw),
        );
        check_row(
            &mut failures,
            row,
            "operational",
            LlmError::Http(diagnostic.clone()).is_operational(),
            main_reference::LlmError::Http(raw.clone()).is_operational(),
        );
        for status in [
            None,
            Some(400),
            Some(413),
            Some(422),
            Some(429),
            Some(500),
            Some(503),
        ] {
            for kind in [
                None,
                Some("context_length_exceeded"),
                Some("invalid_request_error"),
            ] {
                check_row(
                    &mut failures,
                    row,
                    "pressure",
                    crate::context_window::classify_request_pressure_error(status, kind, None, raw),
                    main_reference::classify_request_pressure_error(status, kind, None, raw),
                );
                check_row(
                    &mut failures,
                    row,
                    "context",
                    crate::context_window::classify_context_window_error(
                        status, kind, None, raw, None, None,
                    )
                    .is_some(),
                    main_reference::classify_context_window_error(
                        status, kind, None, raw, None, None,
                    )
                    .is_some(),
                );
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// The redactor is a classifier input (86fbab995's Display and stored text
// called it), so it must be that commit's redactor byte for byte.
#[test]
fn c0_redactor_is_frozen_86fbab995() {
    assert_eq!(
        include_str!("../../archon-observability/src/shared_redactor.rs"),
        include_str!("../../../tests/frozen_86fbab995/shared_redactor.rs")
    );
    assert_eq!(
        include_str!("../../archon-observability/src/redaction.rs"),
        include_str!("../../../tests/frozen_86fbab995/redaction.rs")
    );
}
