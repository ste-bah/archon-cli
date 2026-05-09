use std::collections::HashSet;

use archon_learning::provider_runtime_statuses::ProviderRuntimeStatusSnapshotRecord;
use archon_llm::providers::{AuthFlavor, ProviderDescriptor};
use archon_llm::runtime::{
    AuthProfileSelection, AuthProfileSkipReason, ProviderHealthStatus, ProviderIdentityStatus,
    ProviderRuntimeStatus, redact_provider_metadata,
};

pub(super) fn profile_metadata(
    report: &crate::runtime::provider_auth_selection::ProviderAuthSelectionReport,
) -> serde_json::Value {
    serde_json::json!({
        "selected_profile_id": report
            .selected
            .as_ref()
            .map(|selection| selection.profile.profile_id.clone()),
        "profile_skip_summary": report
            .ordered
            .iter()
            .map(profile_selection_metadata)
            .collect::<Vec<_>>(),
    })
}

fn profile_selection_metadata(selection: &AuthProfileSelection) -> serde_json::Value {
    serde_json::json!({
        "profile_id": selection.profile.profile_id.clone(),
        "auth_kind": selection.profile.auth_kind.clone(),
        "reason": skip_reason_label(selection.reason),
    })
}

pub(super) fn merge_redacted_metadata(
    existing: serde_json::Value,
    incoming: serde_json::Value,
) -> serde_json::Value {
    let mut merged = match existing {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    if let serde_json::Value::Object(map) = incoming {
        merged.extend(map);
    }
    redact_provider_metadata(serde_json::Value::Object(merged))
}

pub(super) fn status_snapshot_record(
    status: &ProviderRuntimeStatus,
) -> ProviderRuntimeStatusSnapshotRecord {
    let mut record = ProviderRuntimeStatusSnapshotRecord::new(
        format!("provider-status-{}", uuid::Uuid::new_v4()),
        status.provider_id.clone(),
        status.runtime_mode.clone(),
        identity_label(status.identity_status),
        health_label(status.health),
        chrono::Utc::now().to_rfc3339(),
    )
    .with_redacted_metadata(status.metadata_redacted_json.clone());
    if let Some(display_name) = &status.display_name {
        record = record.with_display_name(display_name.clone());
    }
    if let Some(profile_id) = &status.profile_id {
        record = record.with_profile(profile_id.clone());
    }
    if let Some(model_id) = &status.model_id {
        record = record.with_model(model_id.clone());
    }
    if let Some(last_success_at) = status.last_success_at {
        record = record.with_last_success(last_success_at.to_rfc3339());
    }
    if let Some(last_failure_at) = status.last_failure_at {
        record = record.with_last_failure(last_failure_at.to_rfc3339());
    }
    for limit in &status.rate_limits {
        record = record.with_rate_limit_id(limit.id.clone());
    }
    record
}

pub(super) fn status_from_descriptor(
    descriptor: &ProviderDescriptor,
    env: &ProviderStatusEnv,
    config: &archon_core::config::ArchonConfig,
) -> ProviderRuntimeStatus {
    let mut status =
        ProviderRuntimeStatus::new(descriptor.id.clone(), runtime_mode(descriptor, config))
            .with_display_name(descriptor.display_name.clone())
            .with_model(descriptor.default_model.clone())
            .with_identity_status(identity_status(descriptor, env, config));
    let health = if credentials_present(descriptor, env) {
        ProviderHealthStatus::Unknown
    } else {
        ProviderHealthStatus::MissingCredentials
    };
    status = status.with_health(health);
    if descriptor.id == "openai-codex" {
        status = status.with_redacted_json(codex_status_metadata(&config.providers.openai_codex));
        if normalize_codex_runtime(&config.providers.openai_codex.runtime) == "app_server" {
            status = status.with_health(ProviderHealthStatus::Unavailable);
        }
    }
    status
}

fn runtime_mode(
    descriptor: &ProviderDescriptor,
    config: &archon_core::config::ArchonConfig,
) -> String {
    if descriptor.id == "openai-codex" {
        normalize_codex_runtime(&config.providers.openai_codex.runtime)
    } else if matches!(descriptor.auth_flavor, AuthFlavor::None) {
        "local".into()
    } else {
        "direct".into()
    }
}

fn codex_status_metadata(config: &archon_core::config::CodexProviderConfig) -> serde_json::Value {
    let discovery = crate::runtime::codex_app_server::discover_codex_app_server(config);
    let mut metadata = discovery.metadata(config);
    if let Some(object) = metadata.as_object_mut() {
        object.insert(
            "codex_strategy".to_string(),
            serde_json::json!({
                "runtime": normalize_codex_runtime(&config.runtime),
                "direct_fallback": config.direct_fallback,
                "adapter_state": "unimplemented",
                "status_note": codex_strategy_status_note(config, discovery.is_configured()),
            }),
        );
    }
    metadata
}

fn codex_strategy_status_note(
    config: &archon_core::config::CodexProviderConfig,
    app_server_configured: bool,
) -> &'static str {
    match (
        normalize_codex_runtime(&config.runtime).as_str(),
        config.direct_fallback,
        app_server_configured,
    ) {
        ("direct", _, true) => "app-server:configured direct-selected",
        ("direct", _, false) => "direct",
        ("auto", true, true) => "app-server:configured direct-fallback",
        ("auto", true, false) => "app-server:not-configured direct-fallback",
        ("auto", false, true) => "app-server:adapter-pending fallback-disabled",
        ("auto", false, false) => "app-server:not-configured fallback-disabled",
        ("app_server", _, true) => "app-server:adapter-pending",
        ("app_server", _, false) => "app-server:not-configured",
        _ => "invalid-codex-runtime",
    }
}

fn normalize_codex_runtime(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

fn identity_status(
    descriptor: &ProviderDescriptor,
    env: &ProviderStatusEnv,
    config: &archon_core::config::ArchonConfig,
) -> ProviderIdentityStatus {
    match descriptor.id.as_str() {
        "anthropic" if env.anthropic_oauth || env.anthropic_bearer_env => {
            ProviderIdentityStatus::Spoof
        }
        "anthropic" => ProviderIdentityStatus::Clean,
        "openai-codex" if runtime_mode(descriptor, config) == "app_server" => {
            ProviderIdentityStatus::AppServer
        }
        "openai-codex" => ProviderIdentityStatus::Custom,
        _ if matches!(descriptor.auth_flavor, AuthFlavor::None) => {
            ProviderIdentityStatus::NotApplicable
        }
        _ => ProviderIdentityStatus::Clean,
    }
}

fn credentials_present(descriptor: &ProviderDescriptor, env: &ProviderStatusEnv) -> bool {
    match descriptor.id.as_str() {
        "anthropic" => env.anthropic_oauth || env.has_env_var(&descriptor.env_key_var),
        "openai-codex" => env.codex_oauth,
        _ if matches!(descriptor.auth_flavor, AuthFlavor::None) => true,
        _ => env.has_env_var(&descriptor.env_key_var),
    }
}

pub(super) fn health_label(health: ProviderHealthStatus) -> &'static str {
    match health {
        ProviderHealthStatus::Healthy => "healthy",
        ProviderHealthStatus::Degraded => "degraded",
        ProviderHealthStatus::Unavailable => "unavailable",
        ProviderHealthStatus::MissingCredentials => "missing-credentials",
        ProviderHealthStatus::Unknown => "unknown-local",
    }
}

pub(super) fn identity_label(identity: ProviderIdentityStatus) -> &'static str {
    match identity {
        ProviderIdentityStatus::Clean => "clean",
        ProviderIdentityStatus::Spoof => "spoof",
        ProviderIdentityStatus::Custom => "custom",
        ProviderIdentityStatus::AppServer => "app-server",
        ProviderIdentityStatus::NotApplicable => "n/a",
    }
}

pub(super) fn status_note(status: &ProviderRuntimeStatus) -> String {
    let exhausted = status.exhausted_limits().len();
    if exhausted > 0 {
        format!("limited:{exhausted}")
    } else if !status.rate_limits.is_empty() {
        format!("recent-limits:{}", status.rate_limits.len())
    } else if let Some(note) = status
        .metadata_redacted_json
        .pointer("/codex_strategy/status_note")
        .and_then(|value| value.as_str())
    {
        note.to_string()
    } else {
        "-".to_string()
    }
}

fn skip_reason_label(reason: AuthProfileSkipReason) -> &'static str {
    match reason {
        AuthProfileSkipReason::Ok => "ok",
        AuthProfileSkipReason::ProfileMissing => "profile-missing",
        AuthProfileSkipReason::ProviderMismatch => "provider-mismatch",
        AuthProfileSkipReason::AuthKindMismatch => "auth-kind-mismatch",
        AuthProfileSkipReason::Expired => "expired",
        AuthProfileSkipReason::RefreshFailed => "refresh-failed",
        AuthProfileSkipReason::RateLimited => "rate-limited",
        AuthProfileSkipReason::UsageLimited => "usage-limited",
        AuthProfileSkipReason::Cooldown => "cooldown",
        AuthProfileSkipReason::Disabled => "disabled",
    }
}

#[derive(Debug, Default)]
pub(super) struct ProviderStatusEnv {
    pub(super) env_vars: HashSet<String>,
    pub(super) anthropic_oauth: bool,
    pub(super) anthropic_bearer_env: bool,
    pub(super) codex_oauth: bool,
}

impl ProviderStatusEnv {
    pub(super) fn detect() -> Self {
        let mut env = Self {
            env_vars: std::env::vars()
                .filter(|(_, value)| !value.is_empty())
                .map(|(key, _)| key)
                .collect(),
            ..Self::default()
        };
        env.anthropic_bearer_env = std::env::var("ANTHROPIC_API_KEY")
            .map(|value| value.starts_with("sk-ant-oat"))
            .unwrap_or(false);
        let path = archon_llm::tokens::credentials_path();
        if let Ok(json) = std::fs::read_to_string(path) {
            env.anthropic_oauth = archon_llm::auth::parse_credentials_json(&json).is_ok();
            env.codex_oauth = archon_llm::auth::parse_codex_credentials_json(&json).is_ok();
        }
        env
    }

    fn has_env_var(&self, name: &str) -> bool {
        !name.is_empty() && self.env_vars.contains(name)
    }
}
