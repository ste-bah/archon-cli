use anyhow::Result;
use archon_learning::provider_runtime_statuses::insert_provider_runtime_status_snapshot;
use archon_learning::runtime_models::ProviderRuntimeEventRecord;
use archon_llm::providers::{list_compat, list_native};
#[cfg(test)]
use archon_llm::runtime::ProviderIdentityStatus;
use archon_llm::runtime::{ProviderHealthStatus, ProviderRuntimeStatus};
use chrono::{DateTime, Utc};
use cozo::DbInstance;
use serde::Serialize;

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
    json: bool,
) -> Result<String> {
    let statuses = collect_and_persist_provider_statuses(provider_filter, config);
    if json {
        render_provider_statuses_json(&statuses)
    } else {
        Ok(render_provider_statuses(&statuses))
    }
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

fn render_provider_statuses_json(statuses: &[ProviderRuntimeStatus]) -> Result<String> {
    let report = ProviderStatusJson {
        generated_at: Utc::now().to_rfc3339(),
        provider_count: statuses.len(),
        providers: statuses,
    };
    Ok(format!("{}\n", serde_json::to_string_pretty(&report)?))
}

#[derive(Debug, Serialize)]
struct ProviderStatusJson<'a> {
    generated_at: String,
    provider_count: usize,
    providers: &'a [ProviderRuntimeStatus],
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
        if !report.ordered.is_empty() {
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
        }

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
        enrich_provider_status_from_events(status, db)?;
    }
    Ok(())
}

fn enrich_provider_status_from_events(
    status: &mut ProviderRuntimeStatus,
    db: &DbInstance,
) -> Result<()> {
    let events = archon_learning::runtime_events::list_provider_runtime_events(
        db,
        Some(&status.provider_id),
    )?;
    if events.is_empty() {
        return Ok(());
    }

    let last_success = events.iter().find(|event| is_success_event(event));
    let last_failure = events.iter().find(|event| is_failure_event(event));
    if let Some(event) = last_success
        && let Some(time) = parse_event_time(&event.created_at)
    {
        status.last_success_at = Some(time);
    }
    if let Some(event) = last_failure
        && let Some(time) = parse_event_time(&event.created_at)
    {
        status.last_failure_at = Some(time);
    }

    apply_event_health(status);
    status.metadata_redacted_json = merge_redacted_metadata(
        status.metadata_redacted_json.clone(),
        event_status_metadata(last_success, last_failure),
    );
    Ok(())
}

fn apply_event_health(status: &mut ProviderRuntimeStatus) {
    if status.health == ProviderHealthStatus::MissingCredentials {
        return;
    }
    match (status.last_success_at, status.last_failure_at) {
        (Some(success), Some(failure)) if failure > success => {
            status.health = ProviderHealthStatus::Degraded;
        }
        (None, Some(_)) => status.health = ProviderHealthStatus::Degraded,
        (Some(_), _) if status.health == ProviderHealthStatus::Unknown => {
            status.health = ProviderHealthStatus::Healthy;
        }
        _ => {}
    }
}

fn event_status_metadata(
    last_success: Option<&ProviderRuntimeEventRecord>,
    last_failure: Option<&ProviderRuntimeEventRecord>,
) -> serde_json::Value {
    serde_json::json!({
        "last_runtime_event": last_success.or(last_failure).map(|event| event.event_id.clone()),
        "last_success_event": last_success.map(|event| event.event_id.clone()),
        "last_failure_event": last_failure.map(|event| event.event_id.clone()),
        "last_failure_reason": last_failure.and_then(|event| event.reason_code.clone()),
        "runtime_event_status_note": event_status_note(last_success, last_failure),
    })
}

fn event_status_note(
    last_success: Option<&ProviderRuntimeEventRecord>,
    last_failure: Option<&ProviderRuntimeEventRecord>,
) -> Option<String> {
    match (last_success, last_failure) {
        (Some(success), Some(failure)) if failure.created_at > success.created_at => Some(format!(
            "last-failure:{}",
            failure
                .reason_code
                .as_deref()
                .unwrap_or(&failure.event_type)
        )),
        (None, Some(failure)) => Some(format!(
            "last-failure:{}",
            failure
                .reason_code
                .as_deref()
                .unwrap_or(&failure.event_type)
        )),
        (Some(_), _) => Some("last-success".to_string()),
        _ => None,
    }
}

fn is_success_event(event: &ProviderRuntimeEventRecord) -> bool {
    event.event_type == "request_succeeded"
}

fn is_failure_event(event: &ProviderRuntimeEventRecord) -> bool {
    matches!(
        event.event_type.as_str(),
        "request_failed" | "token_refresh_failed" | "rate_limit_observed" | "usage_limit_observed"
    )
}

fn parse_event_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

#[cfg(test)]
#[path = "providers_status_tests.rs"]
mod tests;
