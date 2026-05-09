use anyhow::Result;
use archon_learning::provider_runtime_statuses::insert_provider_runtime_status_snapshot;
use archon_llm::providers::{list_compat, list_native};
#[cfg(test)]
use archon_llm::runtime::ProviderIdentityStatus;
use archon_llm::runtime::{ProviderHealthStatus, ProviderRuntimeStatus};
use cozo::DbInstance;

#[path = "providers_status_support.rs"]
mod providers_status_support;
use providers_status_support::{
    ProviderStatusEnv, health_label, identity_label, merge_redacted_metadata, profile_metadata,
    status_from_descriptor, status_note, status_snapshot_record,
};

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
    render_provider_statuses(&collect_and_persist_provider_statuses(
        provider_filter,
        config,
    ))
}

pub(crate) fn collect_and_persist_provider_statuses(
    provider_filter: Option<&str>,
    config: &archon_core::config::ArchonConfig,
) -> Vec<ProviderRuntimeStatus> {
    let mut statuses =
        local_provider_statuses(provider_filter, &ProviderStatusEnv::detect(), config);
    if let Err(error) = enrich_provider_statuses_from_store(&mut statuses) {
        tracing::warn!(%error, "provider status profile enrichment failed");
    }
    if let Err(error) = persist_provider_status_snapshots(&statuses) {
        tracing::warn!(%error, "provider status snapshot persistence failed");
    }
    statuses
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
        status.metadata_redacted_json = merge_redacted_metadata(
            status.metadata_redacted_json.clone(),
            profile_metadata(&report),
        );

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

#[cfg(test)]
#[path = "providers_status_tests.rs"]
mod tests;
