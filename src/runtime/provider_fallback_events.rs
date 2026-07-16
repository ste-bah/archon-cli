//! Persist explicit provider fallback decisions.

use super::learning_store;
use archon_llm::runtime::{
    ProviderRuntimeEvent, ProviderRuntimeEventType, ProviderRuntimeSeverity,
};

pub(crate) fn record_provider_fallback_selected(
    provider_id: &str,
    from_runtime_mode: &str,
    to_runtime_mode: &str,
    reason_code: &str,
    metadata: serde_json::Value,
) {
    record_provider_fallback_decision(
        provider_id,
        from_runtime_mode,
        to_runtime_mode,
        ProviderRuntimeEventType::FallbackSelected,
        ProviderRuntimeSeverity::Warn,
        reason_code,
        metadata,
    );
}

pub(crate) fn record_provider_fallback_denied(
    provider_id: &str,
    from_runtime_mode: &str,
    to_runtime_mode: &str,
    reason_code: &str,
    metadata: serde_json::Value,
) {
    record_provider_fallback_decision(
        provider_id,
        from_runtime_mode,
        to_runtime_mode,
        ProviderRuntimeEventType::FallbackDenied,
        ProviderRuntimeSeverity::Error,
        reason_code,
        metadata,
    );
}

pub(crate) fn record_provider_construction_fallback_denied(
    requested_provider: &str,
    target_provider: &str,
    reason_code: &str,
    metadata: serde_json::Value,
) {
    let db = match learning_store::acquire_default() {
        Ok(db) => db,
        Err(error) => {
            tracing::warn!(
                %error,
                provider = target_provider,
                event_action = "construction_fallback_denied",
                "provider fallback event persistence unavailable"
            );
            return;
        }
    };
    let event = ProviderRuntimeEvent::new(
        target_provider,
        "direct",
        ProviderRuntimeEventType::FallbackDenied,
        ProviderRuntimeSeverity::Error,
    )
    .with_reason(reason_code)
    .with_fallback(requested_provider, target_provider)
    .with_redacted_json(metadata);
    let record = crate::runtime::provider_event_record::provider_event_record(event);
    if let Err(error) = archon_learning::runtime_events::insert_provider_runtime_event(&db, &record)
    {
        tracing::warn!(%error, target_provider, "provider fallback denial persistence failed");
    }
}

fn record_provider_fallback_decision(
    provider_id: &str,
    from_runtime_mode: &str,
    to_runtime_mode: &str,
    event_type: ProviderRuntimeEventType,
    severity: ProviderRuntimeSeverity,
    reason_code: &str,
    metadata: serde_json::Value,
) {
    let db = match learning_store::acquire_default() {
        Ok(db) => db,
        Err(error) => {
            tracing::warn!(
                %error,
                provider = provider_id,
                event_action = ?event_type,
                "provider fallback event persistence unavailable"
            );
            return;
        }
    };
    let event = ProviderRuntimeEvent::new(provider_id, to_runtime_mode, event_type, severity)
        .with_reason(reason_code)
        .with_fallback(from_runtime_mode, to_runtime_mode)
        .with_redacted_json(metadata);
    let record = crate::runtime::provider_event_record::provider_event_record(event);
    if let Err(error) = archon_learning::runtime_events::insert_provider_runtime_event(&db, &record)
    {
        tracing::warn!(%error, provider_id, "provider fallback event persistence failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_fallback_denial_event_uses_provider_fallback_fields() {
        let event = ProviderRuntimeEvent::new(
            "anthropic",
            "direct",
            ProviderRuntimeEventType::FallbackDenied,
            ProviderRuntimeSeverity::Error,
        )
        .with_reason("anthropic_fallback_auth_unavailable")
        .with_fallback("openai", "anthropic");
        let record = crate::runtime::provider_event_record::provider_event_record(event);

        assert_eq!(record.provider_id, "anthropic");
        assert_eq!(record.event_type, "fallback_denied");
        assert_eq!(record.fallback_from.as_deref(), Some("openai"));
        assert_eq!(record.fallback_to.as_deref(), Some("anthropic"));
        assert_eq!(
            record.reason_code.as_deref(),
            Some("anthropic_fallback_auth_unavailable")
        );
    }
}
