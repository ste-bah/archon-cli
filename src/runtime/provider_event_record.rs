//! Conversion from in-process provider runtime events to Cozo records.

use archon_learning::runtime_models::ProviderRuntimeEventRecord;
use archon_llm::runtime::{
    ProviderRuntimeEvent, ProviderRuntimeEventType, ProviderRuntimeSeverity,
};

pub(crate) fn provider_event_record(event: ProviderRuntimeEvent) -> ProviderRuntimeEventRecord {
    let mut record = ProviderRuntimeEventRecord::new(
        event.event_id,
        event.provider_id,
        event.runtime_mode,
        event_type_label(event.event_type),
        severity_label(event.severity),
        event.created_at.to_rfc3339(),
    )
    .with_redacted_json(event.raw_redacted_json);
    if let Some(profile_id) = event.profile_id {
        record = record.with_profile(profile_id);
    }
    if let Some(model_id) = event.model_id {
        record = record.with_model(model_id);
    }
    if let Some(reason_code) = event.reason_code {
        record = record.with_reason(reason_code);
    }
    if let Some(message) = event.message {
        record = record.with_message(message);
    }
    if let Some(retry_count) = event.retry_count {
        record = record.with_retry_count(retry_count);
    }
    if let (Some(from), Some(to)) = (event.fallback_from, event.fallback_to) {
        record = record.with_fallback(from, to);
    }
    if let Some(request_id) = event.request_id {
        record = record.with_request_id(request_id);
    }
    if let Some(run_id) = event.run_id {
        record = record.with_run_id(run_id);
    }
    if let Some(pipeline_id) = event.pipeline_id {
        record = record.with_pipeline_id(pipeline_id);
    }
    record
}

fn event_type_label(event_type: ProviderRuntimeEventType) -> &'static str {
    match event_type {
        ProviderRuntimeEventType::RequestStarted => "request_started",
        ProviderRuntimeEventType::RequestSucceeded => "request_succeeded",
        ProviderRuntimeEventType::RequestFailed => "request_failed",
        ProviderRuntimeEventType::TokenRefreshed => "token_refreshed",
        ProviderRuntimeEventType::TokenRefreshFailed => "token_refresh_failed",
        ProviderRuntimeEventType::RateLimitObserved => "rate_limit_observed",
        ProviderRuntimeEventType::UsageLimitObserved => "usage_limit_observed",
        ProviderRuntimeEventType::ProfileCooldownStarted => "profile_cooldown_started",
        ProviderRuntimeEventType::ProfileCooldownCleared => "profile_cooldown_cleared",
        ProviderRuntimeEventType::FallbackSelected => "fallback_selected",
        ProviderRuntimeEventType::FallbackDenied => "fallback_denied",
        ProviderRuntimeEventType::SpoofIdentitySelected => "spoof_identity_selected",
        ProviderRuntimeEventType::SpoofIdentityRejected => "spoof_identity_rejected",
    }
}

fn severity_label(severity: ProviderRuntimeSeverity) -> &'static str {
    match severity {
        ProviderRuntimeSeverity::Debug => "debug",
        ProviderRuntimeSeverity::Info => "info",
        ProviderRuntimeSeverity::Warn => "warn",
        ProviderRuntimeSeverity::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_provider_event_to_learning_record() {
        let record = provider_event_record(
            ProviderRuntimeEvent::new(
                "anthropic",
                "direct",
                ProviderRuntimeEventType::RequestStarted,
                ProviderRuntimeSeverity::Debug,
            )
            .with_model("claude-sonnet-4-6")
            .with_redacted_json(serde_json::json!({"authorization": "secret"})),
        );

        assert_eq!(record.event_type, "request_started");
        assert_eq!(record.severity, "debug");
        assert_eq!(record.model_id.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(record.raw_redacted_json["authorization"], "[redacted]");
    }
}
