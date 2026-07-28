use std::sync::Arc;

use cozo::DbInstance;

use super::{ObservedRequest, ProviderRuntimeEventRecorder, base_event};
use archon_llm::runtime::{ProviderRuntimeEventType, ProviderRuntimeSeverity};

pub(crate) fn runtime_mode_for_provider_name(provider_name: &str) -> &'static str {
    match provider_name {
        "openai-codex" => "auto",
        "local" => "local",
        _ => "direct",
    }
}

pub(crate) async fn record_provider_fallback(
    requested_provider: &str,
    selected_provider: &str,
    runtime_mode: &str,
    reason_code: &str,
) {
    if requested_provider == selected_provider {
        return;
    }
    let event = base_event(
        selected_provider,
        runtime_mode,
        ProviderRuntimeEventType::FallbackSelected,
        ProviderRuntimeSeverity::Warn,
    )
    .with_reason(reason_code)
    .with_fallback(requested_provider, selected_provider)
    .with_redacted_json(serde_json::json!({
        "requested_provider": requested_provider,
        "selected_provider": selected_provider,
        "source": "provider_construction"
    }));
    ProviderRuntimeEventRecorder::default_learning_store()
        .await
        .record(event)
        .await;
}

pub(super) fn record_agent_provider_incident_sync(
    db: Option<&Arc<DbInstance>>,
    provider_id: &str,
    provider_event_id: &str,
    request: &ObservedRequest,
    reason_code: &str,
) {
    super::super::provider_incident_ledger::record_provider_incident(
        super::super::provider_incident_ledger::ProviderIncidentLedgerInput {
            db,
            agent_type: request.agent_type.as_deref(),
            agent_version: request.agent_version.as_deref(),
            run_id: request.run_id.as_deref(),
            model_id: &request.model,
            provider_id,
            provider_event_id,
            reason_code,
        },
    );
}
