// The baseline text is 86fbab995's Display, annotation for annotation: every
// field that Display passed through `diagnostic_message` (redact_text) is
// redacted here too; ProviderNotFound printed its fields as given. A
// characterization guard pins today's redactor to the frozen 86fbab995 copy
// in tests/frozen_86fbab995, so `red` is that commit's redaction.
fn characterization_variants(raw: &str) -> Vec<(&'static str, LlmError, String)> {
    let red = archon_observability::redaction::redact_text(raw);
    let red = red.as_str();
    let mut rows = vec![
        (
            "http",
            LlmError::Http(raw.into()),
            format!("HTTP error: {red}"),
        ),
        (
            "transport",
            LlmError::Transport(raw.into()),
            format!("HTTP error: {red}"),
        ),
        (
            "auth",
            LlmError::Auth(raw.into()),
            format!("authentication error: {red}"),
        ),
        (
            "serialize",
            LlmError::Serialize(raw.into()),
            format!("serialization error: {red}"),
        ),
        (
            "unsupported",
            LlmError::Unsupported(raw.into()),
            format!("feature not supported: {red}"),
        ),
        (
            "quota",
            LlmError::QuotaExceeded(raw.into()),
            format!("quota exceeded: {red}"),
        ),
        (
            "context",
            LlmError::ContextWindowExceeded {
                provider_message: raw.into(),
                provider: None,
                model: None,
            },
            format!("context window exceeded: {red}"),
        ),
        (
            "overloaded",
            LlmError::Overloaded {
                message: raw.into(),
                status: Some(503),
                retry_after_secs: None,
            },
            format!("server overloaded: {red}"),
        ),
        (
            "rejected",
            LlmError::Rejected {
                error_type: "invalid_request_error".into(),
                message: raw.into(),
            },
            format!("provider rejected the request (invalid_request_error): {red}"),
        ),
        (
            "provider_name",
            LlmError::ProviderNotFound {
                name: raw.into(),
                available: "example".into(),
            },
            format!("provider '{raw}' not found — available: [example]"),
        ),
        (
            "provider_available",
            LlmError::ProviderNotFound {
                name: "example".into(),
                available: raw.into(),
            },
            format!("provider 'example' not found — available: [{raw}]"),
        ),
    ];
    // Varied rejection kinds exercise main's joined and displayed kind + body.
    for (kind, error_type) in [
        ("rejected/context", "context"),
        ("rejected/tool_result", "tool_result"),
        ("rejected/tool", "tool"),
        (
            "rejected/context_length_exceeded",
            "context_length_exceeded",
        ),
        ("rejected/http503", "HTTP 503"),
        ("rejected/rate_limit_error", "rate_limit_error"),
        ("rejected/no_progress", "no-progress stop:"),
        ("rejected/overloaded_error", "overloaded_error"),
        ("rejected/timeout", "timeout"),
    ] {
        rows.push((
            kind,
            LlmError::Rejected {
                error_type: error_type.into(),
                message: raw.into(),
            },
            format!(
                "provider rejected the request ({}): {red}",
                archon_observability::redaction::redact_text(error_type)
            ),
        ));
    }
    for (kind, status) in [
        ("server400", 400),
        ("server429", 429),
        ("server500", 500),
        ("server503", 503),
    ] {
        rows.push((
            kind,
            LlmError::Server {
                status,
                message: raw.into(),
                pipeline_retry_after_secs: None,
            },
            format!("server error ({status}): {red}"),
        ));
    }
    for (kind, wait) in [("rate_limited", 0), ("rate_limited_wait1429", 1429)] {
        rows.push((
            kind,
            LlmError::RateLimited {
                message: raw.into(),
                retry_after_secs: wait,
                from_provider: true,
                pipeline_retry_after_secs: None,
            },
            format!("rate limited: retry after {wait}s: {red}"),
        ));
    }
    // Aborted has no body; exercise it once rather than duplicating it for every body.
    if raw == "context window exceeded: request timed out" {
        rows.push(("aborted", LlmError::Aborted, "request aborted".into()));
    }
    rows
}
