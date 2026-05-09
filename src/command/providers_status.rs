use std::collections::HashSet;

use anyhow::Result;
use archon_learning::provider_runtime_statuses::{
    ProviderRuntimeStatusSnapshotRecord, insert_provider_runtime_status_snapshot,
};
use archon_llm::providers::{AuthFlavor, ProviderDescriptor, list_compat, list_native};
use archon_llm::runtime::{
    AuthProfileSelection, AuthProfileSkipReason, ProviderHealthStatus, ProviderIdentityStatus,
    ProviderRuntimeStatus, redact_provider_metadata,
};
use cozo::DbInstance;

pub(crate) fn render_provider_status(provider_filter: Option<&str>) -> String {
    render_provider_status_with_config(
        provider_filter,
        &archon_core::config::ArchonConfig::default(),
    )
}

pub(crate) fn render_provider_status_with_config(
    provider_filter: Option<&str>,
    config: &archon_core::config::ArchonConfig,
) -> String {
    render_provider_statuses(&local_provider_statuses(
        provider_filter,
        &ProviderStatusEnv::detect(),
        config,
    ))
}

pub(crate) fn render_and_persist_provider_status(
    provider_filter: Option<&str>,
    config: &archon_core::config::ArchonConfig,
) -> String {
    let mut statuses =
        local_provider_statuses(provider_filter, &ProviderStatusEnv::detect(), config);
    if let Err(error) = enrich_provider_statuses_from_store(&mut statuses) {
        tracing::warn!(%error, "provider status profile enrichment failed");
    }
    if let Err(error) = persist_provider_status_snapshots(&statuses) {
        tracing::warn!(%error, "provider status snapshot persistence failed");
    }
    render_provider_statuses(&statuses)
}

#[cfg(test)]
fn render_provider_status_with_env(
    provider_filter: Option<&str>,
    env: &ProviderStatusEnv,
) -> String {
    render_provider_status_with_env_and_config(
        provider_filter,
        env,
        &archon_core::config::ArchonConfig::default(),
    )
}

#[cfg(test)]
fn render_provider_status_with_env_and_config(
    provider_filter: Option<&str>,
    env: &ProviderStatusEnv,
    config: &archon_core::config::ArchonConfig,
) -> String {
    render_provider_statuses(&local_provider_statuses(provider_filter, env, config))
}

fn local_provider_statuses(
    provider_filter: Option<&str>,
    env: &ProviderStatusEnv,
    config: &archon_core::config::ArchonConfig,
) -> Vec<ProviderRuntimeStatus> {
    let mut descriptors = list_native();
    descriptors.extend(list_compat());
    descriptors.sort_by(|a, b| a.id.cmp(&b.id));

    descriptors
        .into_iter()
        .filter(|descriptor| provider_filter.map_or(true, |filter| descriptor.id == filter))
        .map(|descriptor| status_from_descriptor(descriptor, env, config))
        .collect()
}

fn render_provider_statuses(statuses: &[ProviderRuntimeStatus]) -> String {
    let mut out = String::new();
    out.push_str("Provider runtime status (local configuration)\n\n");
    if statuses.is_empty() {
        out.push_str("No provider matched the requested filter.\n");
        return out;
    }
    out.push_str(
        "provider             health               mode        identity    profile              model               notes\n",
    );
    out.push_str(
        "----------------------------------------------------------------------------------------------------------------\n",
    );
    for status in statuses {
        out.push_str(&format!(
            "{:<20} {:<20} {:<11} {:<11} {:<20} {:<19} {}\n",
            status.provider_id,
            health_label(status.health),
            status.runtime_mode,
            identity_label(status.identity_status),
            status.profile_id.as_deref().unwrap_or("-"),
            status.model_id.as_deref().unwrap_or("n/a"),
            status_note(status),
        ));
    }
    out.push_str(
        "\nThis status is local and redacted; use `archon providers doctor --live` for opt-in endpoint checks.\n",
    );
    out
}

fn persist_provider_status_snapshots(statuses: &[ProviderRuntimeStatus]) -> Result<()> {
    if statuses.is_empty() {
        return Ok(());
    }
    let db_path = learning_db_path()?;
    let db = open_learning_db(&db_path)?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    for status in statuses {
        let record = status_snapshot_record(status);
        insert_provider_runtime_status_snapshot(&db, &record)?;
    }
    Ok(())
}

fn learning_db_path() -> Result<std::path::PathBuf> {
    let base = archon_session::storage::default_db_path();
    let parent = base
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    Ok(parent.join("learning.db"))
}

fn open_learning_db(path: &std::path::Path) -> Result<DbInstance> {
    let path_str = path.to_string_lossy().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    DbInstance::new("sqlite", &path_str, "").map_err(|e| anyhow::anyhow!("open learning db: {e}"))
}

fn enrich_provider_statuses_from_store(statuses: &mut [ProviderRuntimeStatus]) -> Result<()> {
    if statuses.is_empty() {
        return Ok(());
    }
    let db_path = learning_db_path()?;
    let db = open_learning_db(&db_path)?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    enrich_provider_statuses_from_db(statuses, &db)
}

fn enrich_provider_statuses_from_db(
    statuses: &mut [ProviderRuntimeStatus],
    db: &DbInstance,
) -> Result<()> {
    for status in statuses {
        let allowed =
            crate::runtime::provider_auth_selection::default_auth_kinds(&status.provider_id);
        let report = crate::runtime::provider_auth_selection::select_provider_auth_profile_from_db(
            db,
            &status.provider_id,
            &allowed,
            None,
        )?;
        if report.ordered.is_empty() {
            continue;
        }
        if let Some(selection) = &report.selected {
            status.profile_id = Some(selection.profile.profile_id.clone());
            if status.health == ProviderHealthStatus::MissingCredentials {
                status.health = ProviderHealthStatus::Unknown;
            }
        } else {
            status.health = ProviderHealthStatus::Degraded;
        }
        status.metadata_redacted_json = redact_provider_metadata(profile_metadata(&report));

        let rate_limits = crate::command::providers_status_limits::recent_rate_limits_from_db(
            db,
            &status.provider_id,
            chrono::Utc::now(),
        )?;
        if !rate_limits.is_empty() {
            if rate_limits.iter().any(|limit| limit.is_exhausted())
                && status.health != ProviderHealthStatus::MissingCredentials
            {
                status.health = ProviderHealthStatus::Degraded;
            }
            status.rate_limits = rate_limits;
        }
    }
    Ok(())
}

fn profile_metadata(
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

fn status_snapshot_record(status: &ProviderRuntimeStatus) -> ProviderRuntimeStatusSnapshotRecord {
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

fn status_from_descriptor(
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
    status
}

fn runtime_mode(
    descriptor: &ProviderDescriptor,
    config: &archon_core::config::ArchonConfig,
) -> String {
    if descriptor.id == "openai-codex" {
        config
            .providers
            .openai_codex
            .runtime
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
    } else if matches!(descriptor.auth_flavor, AuthFlavor::None) {
        "local".into()
    } else {
        "direct".into()
    }
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

fn health_label(health: ProviderHealthStatus) -> &'static str {
    match health {
        ProviderHealthStatus::Healthy => "healthy",
        ProviderHealthStatus::Degraded => "degraded",
        ProviderHealthStatus::Unavailable => "unavailable",
        ProviderHealthStatus::MissingCredentials => "missing-credentials",
        ProviderHealthStatus::Unknown => "unknown-local",
    }
}

fn identity_label(identity: ProviderIdentityStatus) -> &'static str {
    match identity {
        ProviderIdentityStatus::Clean => "clean",
        ProviderIdentityStatus::Spoof => "spoof",
        ProviderIdentityStatus::Custom => "custom",
        ProviderIdentityStatus::AppServer => "app-server",
        ProviderIdentityStatus::NotApplicable => "n/a",
    }
}

fn status_note(status: &ProviderRuntimeStatus) -> String {
    let exhausted = status.exhausted_limits().len();
    if exhausted > 0 {
        format!("limited:{exhausted}")
    } else if !status.rate_limits.is_empty() {
        format!("recent-limits:{}", status.rate_limits.len())
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
struct ProviderStatusEnv {
    env_vars: HashSet<String>,
    anthropic_oauth: bool,
    anthropic_bearer_env: bool,
    codex_oauth: bool,
}

impl ProviderStatusEnv {
    fn detect() -> Self {
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

#[cfg(test)]
#[path = "providers_status_tests.rs"]
mod tests;
