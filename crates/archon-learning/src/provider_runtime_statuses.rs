use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderRuntimeStatusSnapshotRecord {
    pub status_id: String,
    pub provider_id: String,
    pub display_name: Option<String>,
    pub profile_id: Option<String>,
    pub model_id: Option<String>,
    pub runtime_mode: String,
    pub identity_status: String,
    pub health: String,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub rate_limit_ids: Vec<String>,
    pub metadata_redacted_json: serde_json::Value,
    pub observed_at: String,
}

impl ProviderRuntimeStatusSnapshotRecord {
    pub fn new(
        status_id: impl Into<String>,
        provider_id: impl Into<String>,
        runtime_mode: impl Into<String>,
        identity_status: impl Into<String>,
        health: impl Into<String>,
        observed_at: impl Into<String>,
    ) -> Self {
        Self {
            status_id: status_id.into(),
            provider_id: provider_id.into(),
            display_name: None,
            profile_id: None,
            model_id: None,
            runtime_mode: runtime_mode.into(),
            identity_status: identity_status.into(),
            health: health.into(),
            last_success_at: None,
            last_failure_at: None,
            rate_limit_ids: Vec::new(),
            metadata_redacted_json: serde_json::json!({}),
            observed_at: observed_at.into(),
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_profile(mut self, profile_id: impl Into<String>) -> Self {
        self.profile_id = Some(profile_id.into());
        self
    }

    pub fn with_model(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_last_success(mut self, last_success_at: impl Into<String>) -> Self {
        self.last_success_at = Some(last_success_at.into());
        self
    }

    pub fn with_last_failure(mut self, last_failure_at: impl Into<String>) -> Self {
        self.last_failure_at = Some(last_failure_at.into());
        self
    }

    pub fn with_rate_limit_id(mut self, rate_limit_id: impl Into<String>) -> Self {
        let rate_limit_id = rate_limit_id.into();
        if !self.rate_limit_ids.contains(&rate_limit_id) {
            self.rate_limit_ids.push(rate_limit_id);
        }
        self
    }

    pub fn with_redacted_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata_redacted_json = metadata;
        self
    }
}

pub fn insert_provider_runtime_status_snapshot(
    db: &DbInstance,
    status: &ProviderRuntimeStatusSnapshotRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("sid".into(), DataValue::from(status.status_id.as_str()));
    params.insert(
        "provider".into(),
        DataValue::from(status.provider_id.as_str()),
    );
    params.insert(
        "display".into(),
        DataValue::from(status.display_name.as_deref().unwrap_or("")),
    );
    params.insert(
        "profile".into(),
        DataValue::from(status.profile_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "model".into(),
        DataValue::from(status.model_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "runtime".into(),
        DataValue::from(status.runtime_mode.as_str()),
    );
    params.insert(
        "identity".into(),
        DataValue::from(status.identity_status.as_str()),
    );
    params.insert("health".into(), DataValue::from(status.health.as_str()));
    params.insert(
        "success".into(),
        DataValue::from(status.last_success_at.as_deref().unwrap_or("")),
    );
    params.insert(
        "failure".into(),
        DataValue::from(status.last_failure_at.as_deref().unwrap_or("")),
    );
    params.insert(
        "limits".into(),
        DataValue::from(serde_json::to_string(&status.rate_limit_ids)?.as_str()),
    );
    params.insert(
        "metadata".into(),
        DataValue::from(status.metadata_redacted_json.to_string().as_str()),
    );
    params.insert(
        "observed".into(),
        DataValue::from(status.observed_at.as_str()),
    );

    db.run_script(status_put_script(), params, ScriptMutability::Mutable)
        .map_err(|e| anyhow::anyhow!("insert provider_runtime_status_snapshots failed: {e}"))?;
    Ok(())
}

pub fn get_provider_runtime_status_snapshot(
    db: &DbInstance,
    status_id: &str,
) -> Result<Option<ProviderRuntimeStatusSnapshotRecord>> {
    let mut params = BTreeMap::new();
    params.insert("sid".into(), DataValue::from(status_id));
    let result = db
        .run_script(
            status_query("status_id = $sid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get provider_runtime_status_snapshot failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_status(row)))
}

pub fn list_provider_runtime_status_snapshots(
    db: &DbInstance,
    provider_id: &str,
) -> Result<Vec<ProviderRuntimeStatusSnapshotRecord>> {
    let mut params = BTreeMap::new();
    params.insert("provider".into(), DataValue::from(provider_id));
    let result = db
        .run_script(
            status_query("provider_id = $provider"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list provider_runtime_status_snapshots failed: {e}"))?;
    let mut statuses: Vec<_> = result.rows.iter().map(|row| row_to_status(row)).collect();
    statuses.sort_by(|a, b| b.observed_at.cmp(&a.observed_at));
    Ok(statuses)
}

fn status_put_script() -> &'static str {
    "?[status_id, provider_id, display_name, profile_id, model_id, \
     runtime_mode, identity_status, health, last_success_at, \
     last_failure_at, rate_limit_ids_json, metadata_redacted_json, \
     observed_at] <- [[$sid, $provider, $display, $profile, $model, \
     $runtime, $identity, $health, $success, $failure, $limits, \
     $metadata, $observed]] :put provider_runtime_status_snapshots { \
     status_id => provider_id, display_name, profile_id, model_id, \
     runtime_mode, identity_status, health, last_success_at, \
     last_failure_at, rate_limit_ids_json, metadata_redacted_json, \
     observed_at }"
}

fn status_query(predicate: &'static str) -> &'static str {
    match predicate {
        "status_id = $sid" => {
            "?[status_id, provider_id, display_name, profile_id, model_id, \
             runtime_mode, identity_status, health, last_success_at, \
             last_failure_at, rate_limit_ids_json, metadata_redacted_json, \
             observed_at] := *provider_runtime_status_snapshots{status_id, \
             provider_id, display_name, profile_id, model_id, runtime_mode, \
             identity_status, health, last_success_at, last_failure_at, \
             rate_limit_ids_json, metadata_redacted_json, observed_at}, \
             status_id = $sid"
        }
        _ => {
            "?[status_id, provider_id, display_name, profile_id, model_id, \
             runtime_mode, identity_status, health, last_success_at, \
             last_failure_at, rate_limit_ids_json, metadata_redacted_json, \
             observed_at] := *provider_runtime_status_snapshots{status_id, \
             provider_id, display_name, profile_id, model_id, runtime_mode, \
             identity_status, health, last_success_at, last_failure_at, \
             rate_limit_ids_json, metadata_redacted_json, observed_at}, \
             provider_id = $provider"
        }
    }
}

fn row_to_status(row: &[DataValue]) -> ProviderRuntimeStatusSnapshotRecord {
    ProviderRuntimeStatusSnapshotRecord {
        status_id: str_col(row, 0).to_string(),
        provider_id: str_col(row, 1).to_string(),
        display_name: non_empty(str_col(row, 2)),
        profile_id: non_empty(str_col(row, 3)),
        model_id: non_empty(str_col(row, 4)),
        runtime_mode: str_col(row, 5).to_string(),
        identity_status: str_col(row, 6).to_string(),
        health: str_col(row, 7).to_string(),
        last_success_at: non_empty(str_col(row, 8)),
        last_failure_at: non_empty(str_col(row, 9)),
        rate_limit_ids: serde_json::from_str(str_col(row, 10)).unwrap_or_default(),
        metadata_redacted_json: serde_json::from_str(str_col(row, 11))
            .unwrap_or_else(|_| serde_json::json!({})),
        observed_at: str_col(row, 12).to_string(),
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
            "/tmp/test-provider-runtime-statuses-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn provider_runtime_status_snapshot_roundtrips() {
        let db = test_db();
        let status = ProviderRuntimeStatusSnapshotRecord::new(
            "status-1",
            "anthropic",
            "direct",
            "spoof",
            "healthy",
            "2026-05-08T12:00:00Z",
        )
        .with_display_name("Anthropic Claude Code")
        .with_profile("auth-profile-1")
        .with_model("claude-sonnet-4-6")
        .with_last_success("2026-05-08T11:59:00Z")
        .with_rate_limit_id("limit-window-1")
        .with_redacted_metadata(serde_json::json!({"spoof_contract": "ok"}));

        insert_provider_runtime_status_snapshot(&db, &status).unwrap();
        let restored = get_provider_runtime_status_snapshot(&db, "status-1")
            .unwrap()
            .unwrap();

        assert_eq!(restored.provider_id, "anthropic");
        assert_eq!(restored.identity_status, "spoof");
        assert_eq!(restored.rate_limit_ids, vec!["limit-window-1"]);
        assert_eq!(restored.metadata_redacted_json["spoof_contract"], "ok");
    }

    #[test]
    fn provider_runtime_status_snapshots_list_by_provider() {
        let db = test_db();
        insert_provider_runtime_status_snapshot(
            &db,
            &ProviderRuntimeStatusSnapshotRecord::new(
                "status-1",
                "openai-codex",
                "auto",
                "app_server",
                "degraded",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();
        insert_provider_runtime_status_snapshot(
            &db,
            &ProviderRuntimeStatusSnapshotRecord::new(
                "status-2",
                "openai-codex",
                "direct",
                "clean",
                "healthy",
                "2026-05-08T12:01:00Z",
            ),
        )
        .unwrap();

        let statuses = list_provider_runtime_status_snapshots(&db, "openai-codex").unwrap();

        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].status_id, "status-2");
    }
}
