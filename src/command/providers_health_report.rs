//! Provider health reporting from runtime status and Cozo-backed events.

use std::collections::BTreeMap;

use anyhow::Result;
use archon_learning::runtime_models::ProviderRuntimeEventRecord;
use archon_llm::runtime::{ProviderHealthStatus, ProviderIdentityStatus, ProviderRuntimeStatus};
use cozo::DbInstance;
use serde::Serialize;

const RECENT_EVENT_LIMIT: usize = 5;

pub(crate) fn render_provider_health_report(
    provider_filter: Option<&str>,
    config: &archon_core::config::ArchonConfig,
    json: bool,
) -> Result<String> {
    let statuses = crate::command::providers_status::collect_and_persist_provider_statuses(
        provider_filter,
        config,
    );
    let db = open_learning_db(&learning_db_path()?)?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    let events =
        archon_learning::runtime_events::list_provider_runtime_events(&db, provider_filter)?;
    let report =
        ProviderHealthReport::from_records(chrono::Utc::now().to_rfc3339(), &statuses, &events);
    if json {
        Ok(format!("{}\n", serde_json::to_string_pretty(&report)?))
    } else {
        Ok(render_report(&report))
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ProviderHealthReport {
    generated_at: String,
    provider_count: usize,
    providers: Vec<ProviderHealthItem>,
}

impl ProviderHealthReport {
    fn from_records(
        generated_at: String,
        statuses: &[ProviderRuntimeStatus],
        events: &[ProviderRuntimeEventRecord],
    ) -> Self {
        let providers = statuses
            .iter()
            .map(|status| ProviderHealthItem::from_status(status, events))
            .collect::<Vec<_>>();
        Self {
            generated_at,
            provider_count: providers.len(),
            providers,
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ProviderHealthItem {
    provider_id: String,
    display_name: Option<String>,
    health: String,
    runtime_mode: String,
    identity_status: String,
    profile_id: Option<String>,
    model_id: Option<String>,
    rate_limit_count: usize,
    exhausted_rate_limits: usize,
    event_count: usize,
    failure_count: usize,
    fallback_count: usize,
    by_event_type: BTreeMap<String, usize>,
    by_severity: BTreeMap<String, usize>,
    last_failure: Option<ProviderEventItem>,
    last_fallback: Option<ProviderEventItem>,
    recent_events: Vec<ProviderEventItem>,
}

impl ProviderHealthItem {
    fn from_status(status: &ProviderRuntimeStatus, events: &[ProviderRuntimeEventRecord]) -> Self {
        let provider_events = events
            .iter()
            .filter(|event| event.provider_id == status.provider_id)
            .collect::<Vec<_>>();
        let mut by_event_type = BTreeMap::new();
        let mut by_severity = BTreeMap::new();
        for event in &provider_events {
            count(&mut by_event_type, &event.event_type);
            count(&mut by_severity, &event.severity);
        }
        Self {
            provider_id: status.provider_id.clone(),
            display_name: status.display_name.clone(),
            health: health_label(status.health).to_string(),
            runtime_mode: status.runtime_mode.clone(),
            identity_status: identity_label(status.identity_status).to_string(),
            profile_id: status.profile_id.clone(),
            model_id: status.model_id.clone(),
            rate_limit_count: status.rate_limits.len(),
            exhausted_rate_limits: status.exhausted_limits().len(),
            event_count: provider_events.len(),
            failure_count: provider_events
                .iter()
                .filter(|event| is_failure_event(event))
                .count(),
            fallback_count: provider_events
                .iter()
                .filter(|event| is_fallback_event(event))
                .count(),
            by_event_type,
            by_severity,
            last_failure: provider_events
                .iter()
                .find(|event| is_failure_event(event))
                .map(|event| ProviderEventItem::from(*event)),
            last_fallback: provider_events
                .iter()
                .find(|event| is_fallback_event(event))
                .map(|event| ProviderEventItem::from(*event)),
            recent_events: provider_events
                .iter()
                .take(RECENT_EVENT_LIMIT)
                .map(|event| ProviderEventItem::from(*event))
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ProviderEventItem {
    event_id: String,
    event_type: String,
    severity: String,
    reason_code: Option<String>,
    fallback_from: Option<String>,
    fallback_to: Option<String>,
    created_at: String,
}

impl From<&ProviderRuntimeEventRecord> for ProviderEventItem {
    fn from(event: &ProviderRuntimeEventRecord) -> Self {
        Self {
            event_id: event.event_id.clone(),
            event_type: event.event_type.clone(),
            severity: event.severity.clone(),
            reason_code: event.reason_code.clone(),
            fallback_from: event.fallback_from.clone(),
            fallback_to: event.fallback_to.clone(),
            created_at: event.created_at.clone(),
        }
    }
}

fn render_report(report: &ProviderHealthReport) -> String {
    let mut out = String::new();
    out.push_str("Provider health report\n\n");
    if report.providers.is_empty() {
        out.push_str("No provider matched the requested filter.\n");
        return out;
    }
    out.push_str(
        "provider             health               mode        identity    limits  events  failures  fallback  notes\n",
    );
    out.push_str(
        "----------------------------------------------------------------------------------------------------------\n",
    );
    for provider in &report.providers {
        out.push_str(&format!(
            "{:<20} {:<20} {:<11} {:<11} {:<7} {:<7} {:<9} {:<9} {}\n",
            provider.provider_id,
            provider.health,
            provider.runtime_mode,
            provider.identity_status,
            provider.rate_limit_count,
            provider.event_count,
            provider.failure_count,
            provider.fallback_count,
            provider_note(provider),
        ));
    }
    out.push_str("\nEvidence is redacted and sourced from Cozo provider runtime tables.\n");
    out
}

fn provider_note(provider: &ProviderHealthItem) -> String {
    if provider.exhausted_rate_limits > 0 {
        format!("exhausted-limits:{}", provider.exhausted_rate_limits)
    } else if let Some(fallback) = &provider.last_fallback {
        format!(
            "fallback:{}:{}",
            fallback.event_type,
            fallback.reason_code.as_deref().unwrap_or("-")
        )
    } else if let Some(failure) = &provider.last_failure {
        format!(
            "last-failure:{}:{}",
            failure.event_type,
            failure.reason_code.as_deref().unwrap_or("-")
        )
    } else {
        "-".to_string()
    }
}

fn is_failure_event(event: &ProviderRuntimeEventRecord) -> bool {
    let severity = event.severity.to_ascii_lowercase();
    let event_type = event.event_type.to_ascii_lowercase();
    matches!(severity.as_str(), "warn" | "warning" | "error" | "critical")
        || event_type.contains("failed")
        || event_type.contains("limit")
}

fn is_fallback_event(event: &ProviderRuntimeEventRecord) -> bool {
    event
        .event_type
        .to_ascii_lowercase()
        .starts_with("fallback_")
}

fn count(counts: &mut BTreeMap<String, usize>, key: &str) {
    *counts.entry(key.to_string()).or_default() += 1;
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

#[cfg(test)]
#[path = "providers_health_report_tests.rs"]
mod tests;
