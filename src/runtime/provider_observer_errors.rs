use archon_llm::provider::LlmError;
use archon_llm::runtime::{ProviderRuntimeEventType, ProviderRuntimeSeverity};

pub(super) fn error_kind(error: &LlmError) -> &'static str {
    match error {
        LlmError::Http(_) => "http_error",
        LlmError::Auth(_) => "auth_error",
        LlmError::RateLimited { .. } => "rate_limited",
        LlmError::Overloaded => "overloaded",
        LlmError::Server { .. } => "server_error",
        LlmError::Serialize(_) => "serialization_error",
        LlmError::Unsupported(_) => "unsupported_feature",
        LlmError::ProviderNotFound { .. } => "provider_not_found",
        LlmError::QuotaExceeded(_) => "quota_exceeded",
        LlmError::Aborted => "aborted",
        LlmError::ContextWindowExceeded { .. } => "context_window_exceeded",
        _ => "unknown_error",
    }
}

pub(super) fn error_message(error: &LlmError) -> &'static str {
    match error {
        LlmError::RateLimited { .. } => "provider reported a rate limit",
        LlmError::QuotaExceeded(_) => "provider reported a usage or quota limit",
        LlmError::Auth(_) => "provider authentication failed",
        LlmError::Server { .. } => "provider returned a server error",
        LlmError::ProviderNotFound { .. } => "provider was not found",
        LlmError::Unsupported(_) => "provider does not support the requested feature",
        LlmError::Aborted => "provider request was aborted",
        LlmError::Http(_) => "provider HTTP request failed",
        LlmError::Overloaded => "provider reported overload",
        LlmError::Serialize(_) => "provider request or response serialization failed",
        LlmError::ContextWindowExceeded { .. } => "provider context window was exceeded",
        _ => "provider request failed",
    }
}

pub(super) fn error_severity(error: &LlmError) -> ProviderRuntimeSeverity {
    match error {
        LlmError::RateLimited { .. } | LlmError::QuotaExceeded(_) | LlmError::Overloaded => {
            ProviderRuntimeSeverity::Warn
        }
        _ => ProviderRuntimeSeverity::Error,
    }
}

pub(super) fn limit_event_type(error: &LlmError) -> Option<ProviderRuntimeEventType> {
    match error {
        LlmError::RateLimited { .. } => Some(ProviderRuntimeEventType::RateLimitObserved),
        LlmError::QuotaExceeded(_) => Some(ProviderRuntimeEventType::UsageLimitObserved),
        _ => None,
    }
}

pub(super) fn error_metadata(error: &LlmError) -> serde_json::Value {
    match error {
        LlmError::RateLimited { retry_after_secs } => serde_json::json!({
            "error_kind": error_kind(error),
            "retry_after_secs": retry_after_secs,
        }),
        LlmError::Server { status, .. } => serde_json::json!({
            "error_kind": error_kind(error),
            "status": status,
        }),
        _ => serde_json::json!({
            "error_kind": error_kind(error),
        }),
    }
}
