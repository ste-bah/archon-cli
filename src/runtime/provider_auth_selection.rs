//! Cozo-backed provider auth profile selection.

use anyhow::Result;
use archon_learning::provider_auth_profiles::{
    ProviderAuthProfileRecord, list_provider_auth_profiles,
};
use archon_llm::runtime::{
    AuthProfileSelection, AuthProfileSkipReason, AuthProfileSource, ProviderAuthProfile,
    ordered_profiles_for_selection,
};
use chrono::{DateTime, Utc};
use cozo::DbInstance;

#[derive(Debug, Clone)]
pub(crate) struct ProviderAuthSelectionReport {
    pub(crate) provider_id: String,
    pub(crate) selected: Option<AuthProfileSelection>,
    pub(crate) ordered: Vec<AuthProfileSelection>,
}

impl ProviderAuthSelectionReport {
    #[cfg(test)]
    pub(crate) fn skipped(&self) -> impl Iterator<Item = &AuthProfileSelection> {
        let selected_id = self
            .selected
            .as_ref()
            .map(|selection| selection.profile.profile_id.as_str());
        self.ordered
            .iter()
            .filter(move |selection| Some(selection.profile.profile_id.as_str()) != selected_id)
    }
}

pub(crate) fn select_provider_auth_profile_from_db(
    db: &DbInstance,
    provider_id: &str,
    allowed_auth_kinds: &[&str],
    preferred_profile_id: Option<&str>,
) -> Result<ProviderAuthSelectionReport> {
    let records = list_provider_auth_profiles(db, provider_id)?;
    Ok(select_provider_auth_profile_from_records(
        provider_id,
        &records,
        allowed_auth_kinds,
        preferred_profile_id,
        Utc::now(),
    ))
}

pub(crate) fn selected_provider_auth_profile_id(provider_id: &str) -> Option<String> {
    let path = crate::command::store_paths::evidence_db_path(&["ARCHON_LEARNING_DB_PATH"]);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let path_str = path.to_string_lossy().to_string();
    let db = DbInstance::new("sqlite", &path_str, "").ok()?;
    archon_learning::schema::ensure_learning_schema(&db).ok()?;
    let allowed = default_auth_kinds(provider_id);
    select_provider_auth_profile_from_db(&db, provider_id, &allowed, None)
        .ok()?
        .selected
        .map(|selection| selection.profile.profile_id)
}

pub(crate) fn default_auth_kinds(provider_id: &str) -> Vec<&'static str> {
    match provider_id {
        "anthropic" | "openai-codex" => vec!["oauth", "api_key"],
        _ => vec!["api_key"],
    }
}

fn select_provider_auth_profile_from_records(
    provider_id: &str,
    records: &[ProviderAuthProfileRecord],
    allowed_auth_kinds: &[&str],
    preferred_profile_id: Option<&str>,
    now: DateTime<Utc>,
) -> ProviderAuthSelectionReport {
    let profiles: Vec<_> = records
        .iter()
        .map(|record| record_to_runtime_profile(record, now))
        .collect();
    let ordered = ordered_profiles_for_selection(
        &profiles,
        provider_id,
        allowed_auth_kinds,
        preferred_profile_id,
        now,
    );
    let selected = ordered
        .iter()
        .find(|selection| selection.reason == AuthProfileSkipReason::Ok)
        .cloned();
    ProviderAuthSelectionReport {
        provider_id: provider_id.to_string(),
        selected,
        ordered,
    }
}

fn record_to_runtime_profile(
    record: &ProviderAuthProfileRecord,
    now: DateTime<Utc>,
) -> ProviderAuthProfile {
    let mut profile = ProviderAuthProfile::new(
        record.profile_id.clone(),
        record.provider_id.clone(),
        record.auth_kind.clone(),
        source_from_record(&record.source),
    );
    profile.display_name = record.display_name.clone();
    profile.identity_fingerprint = record.identity_fingerprint.clone();
    profile.created_at = parse_time(&record.created_at).unwrap_or(now);
    profile.updated_at = parse_time(&record.updated_at).unwrap_or(now);
    profile.last_used_at = record.last_used_at.as_deref().and_then(parse_time);
    profile.last_good_at = record.last_good_at.as_deref().and_then(parse_time);
    profile.last_failed_at = record.last_failed_at.as_deref().and_then(parse_time);
    profile.failure_count = record.failure_count.max(0) as u32;
    profile.cooldown_until = record.cooldown_until.as_deref().and_then(parse_time);
    profile.disabled_reason = record.disabled_reason.clone();
    profile.metadata_json = record.metadata_redacted_json.clone();
    profile
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn source_from_record(source: &str) -> AuthProfileSource {
    match source {
        "archon_store" => AuthProfileSource::ArchonStore,
        "config" => AuthProfileSource::Config,
        "env" => AuthProfileSource::Env,
        "external_codex" | "external_codex_cli" => AuthProfileSource::ExternalCodex,
        "aws_chain" => AuthProfileSource::AwsChain,
        "gcp_credentials" => AuthProfileSource::GcpCredentials,
        "local_runtime" => AuthProfileSource::LocalRuntime,
        _ => AuthProfileSource::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn record(id: &str) -> ProviderAuthProfileRecord {
        ProviderAuthProfileRecord::new(id, "anthropic", "oauth", "archon_store", now())
    }

    fn now() -> String {
        "2026-05-08T12:00:00Z".to_string()
    }

    #[test]
    fn selects_healthy_profile_before_cooldown() {
        let now = parse_time("2026-05-08T12:00:00Z").unwrap();
        let healthy = record("healthy");
        let cooldown =
            record("cooldown").with_cooldown((now + Duration::hours(1)).to_rfc3339(), "quota");

        let report = select_provider_auth_profile_from_records(
            "anthropic",
            &[cooldown, healthy],
            &["oauth"],
            None,
            now,
        );

        assert_eq!(
            report
                .selected
                .as_ref()
                .unwrap()
                .profile
                .profile_id
                .as_str(),
            "healthy"
        );
        assert_eq!(report.skipped().count(), 1);
    }

    #[test]
    fn preferred_profile_wins_when_healthy() {
        let now = parse_time("2026-05-08T12:00:00Z").unwrap();
        let preferred = record("preferred");
        let other = record("other");

        let report = select_provider_auth_profile_from_records(
            "anthropic",
            &[other, preferred],
            &["oauth"],
            Some("preferred"),
            now,
        );

        assert_eq!(
            report
                .selected
                .as_ref()
                .unwrap()
                .profile
                .profile_id
                .as_str(),
            "preferred"
        );
    }

    #[test]
    fn auth_kind_mismatch_prevents_selection() {
        let now = parse_time("2026-05-08T12:00:00Z").unwrap();
        let profile = ProviderAuthProfileRecord::new(
            "api-key",
            "anthropic",
            "api_key",
            "env",
            now.to_rfc3339(),
        );

        let report = select_provider_auth_profile_from_records(
            "anthropic",
            &[profile],
            &["oauth"],
            None,
            now,
        );

        assert!(report.selected.is_none());
        assert_eq!(
            report.ordered[0].reason,
            AuthProfileSkipReason::AuthKindMismatch
        );
    }
}
