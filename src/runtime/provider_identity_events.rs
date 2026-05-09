//! Provider identity runtime events.

use archon_llm::identity::IdentityMode;
use archon_llm::provider::LlmProvider;
use archon_llm::runtime::{
    ProviderIdentityStatus, ProviderRuntimeEventType, ProviderRuntimeSeverity,
};

use super::{ProviderRuntimeEventRecorder, base_event};

pub(super) fn record_provider_identity_decision(
    recorder: &ProviderRuntimeEventRecorder,
    provider: &dyn LlmProvider,
    runtime_mode: &str,
    profile_id: Option<&str>,
) {
    let Some(client) = provider.as_anthropic() else {
        return;
    };

    let (event_type, severity, reason, message) = match &client.identity().mode {
        IdentityMode::Spoof { .. } => (
            ProviderRuntimeEventType::SpoofIdentitySelected,
            ProviderRuntimeSeverity::Info,
            "spoof_identity_active",
            "Anthropic Claude Code spoof identity is active",
        ),
        IdentityMode::Clean => (
            ProviderRuntimeEventType::SpoofIdentityRejected,
            ProviderRuntimeSeverity::Debug,
            "clean_identity",
            "Anthropic clean identity is active",
        ),
        IdentityMode::Custom { .. } => (
            ProviderRuntimeEventType::SpoofIdentityRejected,
            ProviderRuntimeSeverity::Debug,
            "custom_identity",
            "Anthropic custom identity is active",
        ),
    };

    let mut event = base_event(provider.name(), runtime_mode, event_type, severity)
        .with_reason(reason)
        .with_message(message)
        .with_redacted_json(serde_json::json!({
            "identity_status": identity_status_label(identity_status_for_provider(provider)),
            "selection_source": "anthropic_identity_provider",
        }));
    if let Some(profile_id) = profile_id {
        event = event.with_profile(profile_id.to_string());
    }
    recorder.record(event);
}

pub(super) fn identity_status_for_provider(provider: &dyn LlmProvider) -> ProviderIdentityStatus {
    match provider
        .as_anthropic()
        .map(|client| &client.identity().mode)
    {
        Some(IdentityMode::Spoof { .. }) => ProviderIdentityStatus::Spoof,
        Some(IdentityMode::Clean) => ProviderIdentityStatus::Clean,
        Some(IdentityMode::Custom { .. }) => ProviderIdentityStatus::Custom,
        None => ProviderIdentityStatus::NotApplicable,
    }
}

pub(super) fn identity_status_label(identity_status: ProviderIdentityStatus) -> &'static str {
    match identity_status {
        ProviderIdentityStatus::Clean => "clean",
        ProviderIdentityStatus::Spoof => "spoof",
        ProviderIdentityStatus::Custom => "custom",
        ProviderIdentityStatus::AppServer => "app_server",
        ProviderIdentityStatus::NotApplicable => "n/a",
    }
}
