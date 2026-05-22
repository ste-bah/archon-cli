use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::cozo_guard::run_script_guarded;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderAuthProfileRecord {
    pub profile_id: String,
    pub provider_id: String,
    pub auth_kind: String,
    pub display_name: Option<String>,
    pub source: String,
    pub identity_fingerprint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: Option<String>,
    pub last_good_at: Option<String>,
    pub last_failed_at: Option<String>,
    pub failure_count: i64,
    pub cooldown_until: Option<String>,
    pub disabled_reason: Option<String>,
    pub metadata_redacted_json: serde_json::Value,
}

impl ProviderAuthProfileRecord {
    pub fn new(
        profile_id: impl Into<String>,
        provider_id: impl Into<String>,
        auth_kind: impl Into<String>,
        source: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        let created_at = created_at.into();
        Self {
            profile_id: profile_id.into(),
            provider_id: provider_id.into(),
            auth_kind: auth_kind.into(),
            display_name: None,
            source: source.into(),
            identity_fingerprint: None,
            created_at: created_at.clone(),
            updated_at: created_at,
            last_used_at: None,
            last_good_at: None,
            last_failed_at: None,
            failure_count: 0,
            cooldown_until: None,
            disabled_reason: None,
            metadata_redacted_json: serde_json::json!({}),
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_identity_fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.identity_fingerprint = Some(fingerprint.into());
        self
    }

    pub fn with_usage(
        mut self,
        last_used_at: Option<String>,
        last_good_at: Option<String>,
        last_failed_at: Option<String>,
        failure_count: i64,
    ) -> Self {
        self.last_used_at = last_used_at;
        self.last_good_at = last_good_at;
        self.last_failed_at = last_failed_at;
        self.failure_count = failure_count.max(0);
        self
    }

    pub fn with_cooldown(
        mut self,
        cooldown_until: impl Into<String>,
        disabled_reason: impl Into<String>,
    ) -> Self {
        self.cooldown_until = Some(cooldown_until.into());
        self.disabled_reason = Some(disabled_reason.into());
        self
    }

    pub fn with_disabled_reason(mut self, disabled_reason: impl Into<String>) -> Self {
        self.disabled_reason = Some(disabled_reason.into());
        self
    }

    pub fn with_redacted_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata_redacted_json = metadata;
        self
    }
}

pub fn insert_provider_auth_profile(
    db: &DbInstance,
    profile: &ProviderAuthProfileRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(profile.profile_id.as_str()));
    params.insert(
        "provider".into(),
        DataValue::from(profile.provider_id.as_str()),
    );
    params.insert("kind".into(), DataValue::from(profile.auth_kind.as_str()));
    params.insert(
        "display".into(),
        DataValue::from(profile.display_name.as_deref().unwrap_or("")),
    );
    params.insert("source".into(), DataValue::from(profile.source.as_str()));
    params.insert(
        "fingerprint".into(),
        DataValue::from(profile.identity_fingerprint.as_deref().unwrap_or("")),
    );
    params.insert(
        "created".into(),
        DataValue::from(profile.created_at.as_str()),
    );
    params.insert(
        "updated".into(),
        DataValue::from(profile.updated_at.as_str()),
    );
    params.insert(
        "used".into(),
        DataValue::from(profile.last_used_at.as_deref().unwrap_or("")),
    );
    params.insert(
        "good".into(),
        DataValue::from(profile.last_good_at.as_deref().unwrap_or("")),
    );
    params.insert(
        "failed".into(),
        DataValue::from(profile.last_failed_at.as_deref().unwrap_or("")),
    );
    params.insert(
        "failure_count".into(),
        DataValue::from(profile.failure_count.max(0)),
    );
    params.insert(
        "cooldown".into(),
        DataValue::from(profile.cooldown_until.as_deref().unwrap_or("")),
    );
    params.insert(
        "disabled".into(),
        DataValue::from(profile.disabled_reason.as_deref().unwrap_or("")),
    );
    params.insert(
        "metadata".into(),
        DataValue::from(profile.metadata_redacted_json.to_string().as_str()),
    );

    run_script_guarded(
        db,
        auth_profile_put_script(),
        params,
        ScriptMutability::Mutable,
        "insert provider_auth_profiles failed",
    )?;
    Ok(())
}

pub fn get_provider_auth_profile(
    db: &DbInstance,
    profile_id: &str,
) -> Result<Option<ProviderAuthProfileRecord>> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(profile_id));
    let result = run_script_guarded(
        db,
        auth_profile_query("profile_id = $pid"),
        params,
        ScriptMutability::Immutable,
        "get provider_auth_profile failed",
    )?;
    Ok(result.rows.first().map(|row| row_to_auth_profile(row)))
}

pub fn list_provider_auth_profiles(
    db: &DbInstance,
    provider_id: &str,
) -> Result<Vec<ProviderAuthProfileRecord>> {
    let mut params = BTreeMap::new();
    params.insert("provider".into(), DataValue::from(provider_id));
    let result = run_script_guarded(
        db,
        auth_profile_query("provider_id = $provider"),
        params,
        ScriptMutability::Immutable,
        "list provider_auth_profiles failed",
    )?;
    let mut profiles: Vec<_> = result
        .rows
        .iter()
        .map(|row| row_to_auth_profile(row))
        .collect();
    profiles.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(profiles)
}

pub fn list_all_provider_auth_profiles(db: &DbInstance) -> Result<Vec<ProviderAuthProfileRecord>> {
    let result = run_script_guarded(
        db,
        auth_profile_query("all"),
        Default::default(),
        ScriptMutability::Immutable,
        "list provider_auth_profiles failed",
    )?;
    let mut profiles: Vec<_> = result
        .rows
        .iter()
        .map(|row| row_to_auth_profile(row))
        .collect();
    profiles.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(profiles)
}

fn auth_profile_put_script() -> &'static str {
    "?[profile_id, provider_id, auth_kind, display_name, source, \
     identity_fingerprint, created_at, updated_at, last_used_at, \
     last_good_at, last_failed_at, failure_count, cooldown_until, \
     disabled_reason, metadata_redacted_json] <- [[$pid, $provider, \
     $kind, $display, $source, $fingerprint, $created, $updated, $used, \
     $good, $failed, $failure_count, $cooldown, $disabled, $metadata]] \
     :put provider_auth_profiles { profile_id => provider_id, auth_kind, \
     display_name, source, identity_fingerprint, created_at, updated_at, \
     last_used_at, last_good_at, last_failed_at, failure_count, \
     cooldown_until, disabled_reason, metadata_redacted_json }"
}

fn auth_profile_query(predicate: &'static str) -> &'static str {
    match predicate {
        "profile_id = $pid" => {
            "?[profile_id, provider_id, auth_kind, display_name, source, \
             identity_fingerprint, created_at, updated_at, last_used_at, \
             last_good_at, last_failed_at, failure_count, cooldown_until, \
             disabled_reason, metadata_redacted_json] := \
             *provider_auth_profiles{profile_id, provider_id, auth_kind, \
             display_name, source, identity_fingerprint, created_at, \
             updated_at, last_used_at, last_good_at, last_failed_at, \
             failure_count, cooldown_until, disabled_reason, \
             metadata_redacted_json}, profile_id = $pid"
        }
        "all" => {
            "?[profile_id, provider_id, auth_kind, display_name, source, \
             identity_fingerprint, created_at, updated_at, last_used_at, \
             last_good_at, last_failed_at, failure_count, cooldown_until, \
             disabled_reason, metadata_redacted_json] := \
             *provider_auth_profiles{profile_id, provider_id, auth_kind, \
             display_name, source, identity_fingerprint, created_at, \
             updated_at, last_used_at, last_good_at, last_failed_at, \
             failure_count, cooldown_until, disabled_reason, \
             metadata_redacted_json}"
        }
        _ => {
            "?[profile_id, provider_id, auth_kind, display_name, source, \
             identity_fingerprint, created_at, updated_at, last_used_at, \
             last_good_at, last_failed_at, failure_count, cooldown_until, \
             disabled_reason, metadata_redacted_json] := \
             *provider_auth_profiles{profile_id, provider_id, auth_kind, \
             display_name, source, identity_fingerprint, created_at, \
             updated_at, last_used_at, last_good_at, last_failed_at, \
             failure_count, cooldown_until, disabled_reason, \
             metadata_redacted_json}, provider_id = $provider"
        }
    }
}

fn row_to_auth_profile(row: &[DataValue]) -> ProviderAuthProfileRecord {
    ProviderAuthProfileRecord {
        profile_id: str_col(row, 0).to_string(),
        provider_id: str_col(row, 1).to_string(),
        auth_kind: str_col(row, 2).to_string(),
        display_name: non_empty(str_col(row, 3)),
        source: str_col(row, 4).to_string(),
        identity_fingerprint: non_empty(str_col(row, 5)),
        created_at: str_col(row, 6).to_string(),
        updated_at: str_col(row, 7).to_string(),
        last_used_at: non_empty(str_col(row, 8)),
        last_good_at: non_empty(str_col(row, 9)),
        last_failed_at: non_empty(str_col(row, 10)),
        failure_count: row[11].get_int().unwrap_or(0),
        cooldown_until: non_empty(str_col(row, 12)),
        disabled_reason: non_empty(str_col(row, 13)),
        metadata_redacted_json: serde_json::from_str(str_col(row, 14))
            .unwrap_or_else(|_| serde_json::json!({})),
    }
}

fn str_col(row: &[DataValue], index: usize) -> &str {
    row[index].get_str().unwrap_or("")
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-provider-auth-profiles-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn provider_auth_profile_roundtrips_without_account_secret() {
        let db = test_db();
        let profile = ProviderAuthProfileRecord::new(
            "auth-profile-1",
            "anthropic",
            "oauth",
            "archon_store",
            "2026-05-08T12:00:00Z",
        )
        .with_display_name("Claude Code OAuth")
        .with_identity_fingerprint("sha256:identity")
        .with_usage(
            Some("2026-05-08T12:02:00Z".to_string()),
            Some("2026-05-08T12:02:00Z".to_string()),
            None,
            0,
        )
        .with_redacted_metadata(serde_json::json!({"spoof": true}));

        insert_provider_auth_profile(&db, &profile).unwrap();
        let restored = get_provider_auth_profile(&db, "auth-profile-1")
            .unwrap()
            .unwrap();

        assert_eq!(restored.provider_id, "anthropic");
        assert_eq!(restored.display_name.as_deref(), Some("Claude Code OAuth"));
        assert_eq!(
            restored.identity_fingerprint.as_deref(),
            Some("sha256:identity")
        );
        assert_eq!(restored.metadata_redacted_json["spoof"], true);
    }

    #[test]
    fn provider_auth_profiles_list_by_provider() {
        let db = test_db();
        insert_provider_auth_profile(
            &db,
            &ProviderAuthProfileRecord::new(
                "auth-profile-1",
                "openai-codex",
                "oauth",
                "external_codex",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();
        insert_provider_auth_profile(
            &db,
            &ProviderAuthProfileRecord::new(
                "auth-profile-2",
                "openai-codex",
                "oauth",
                "external_codex",
                "2026-05-08T12:01:00Z",
            )
            .with_cooldown("2026-05-08T12:30:00Z", "usage_limited"),
        )
        .unwrap();

        let profiles = list_provider_auth_profiles(&db, "openai-codex").unwrap();

        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].profile_id, "auth-profile-2");
        assert_eq!(
            profiles[0].cooldown_until.as_deref(),
            Some("2026-05-08T12:30:00Z")
        );
    }

    #[test]
    fn provider_auth_profiles_list_all_includes_custom_providers() {
        let db = test_db();
        insert_provider_auth_profile(
            &db,
            &ProviderAuthProfileRecord::new(
                "auth-profile-custom",
                "custom-openai-compatible",
                "api_key",
                "config",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();

        let profiles = list_all_provider_auth_profiles(&db).unwrap();

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].provider_id, "custom-openai-compatible");
    }
}
