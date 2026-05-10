use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderRateLimitWindowRecord {
    pub window_id: String,
    pub provider_id: String,
    pub profile_id: Option<String>,
    pub model_id: Option<String>,
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub window_kind: String,
    pub used_percent: Option<f64>,
    pub resets_at: Option<String>,
    pub raw_redacted_json: serde_json::Value,
    pub observed_at: String,
}

impl ProviderRateLimitWindowRecord {
    pub fn new(
        window_id: impl Into<String>,
        provider_id: impl Into<String>,
        window_kind: impl Into<String>,
        observed_at: impl Into<String>,
    ) -> Self {
        Self {
            window_id: window_id.into(),
            provider_id: provider_id.into(),
            profile_id: None,
            model_id: None,
            limit_id: None,
            limit_name: None,
            window_kind: window_kind.into(),
            used_percent: None,
            resets_at: None,
            raw_redacted_json: serde_json::json!({}),
            observed_at: observed_at.into(),
        }
    }

    pub fn with_profile(mut self, profile_id: impl Into<String>) -> Self {
        self.profile_id = Some(profile_id.into());
        self
    }

    pub fn with_model(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_limit(
        mut self,
        limit_id: impl Into<String>,
        limit_name: impl Into<String>,
    ) -> Self {
        self.limit_id = Some(limit_id.into());
        self.limit_name = Some(limit_name.into());
        self
    }

    pub fn with_used_percent(mut self, used_percent: f64) -> Self {
        self.used_percent = Some(used_percent.clamp(0.0, 100.0));
        self
    }

    pub fn with_resets_at(mut self, resets_at: impl Into<String>) -> Self {
        self.resets_at = Some(resets_at.into());
        self
    }

    pub fn with_redacted_json(mut self, raw_redacted_json: serde_json::Value) -> Self {
        self.raw_redacted_json = raw_redacted_json;
        self
    }

    pub fn is_exhausted(&self) -> bool {
        self.used_percent
            .map(|used_percent| used_percent >= 100.0)
            .unwrap_or(false)
    }
}

pub fn insert_provider_rate_limit_window(
    db: &DbInstance,
    window: &ProviderRateLimitWindowRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("wid".into(), DataValue::from(window.window_id.as_str()));
    params.insert(
        "provider".into(),
        DataValue::from(window.provider_id.as_str()),
    );
    params.insert(
        "profile".into(),
        DataValue::from(window.profile_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "model".into(),
        DataValue::from(window.model_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "limit_id".into(),
        DataValue::from(window.limit_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "limit_name".into(),
        DataValue::from(window.limit_name.as_deref().unwrap_or("")),
    );
    params.insert("kind".into(), DataValue::from(window.window_kind.as_str()));
    params.insert(
        "used".into(),
        DataValue::from(optional_percent(window.used_percent)),
    );
    params.insert(
        "resets".into(),
        DataValue::from(window.resets_at.as_deref().unwrap_or("")),
    );
    params.insert(
        "raw".into(),
        DataValue::from(window.raw_redacted_json.to_string().as_str()),
    );
    params.insert(
        "observed".into(),
        DataValue::from(window.observed_at.as_str()),
    );

    db.run_script(rate_limit_put_script(), params, ScriptMutability::Mutable)
        .map_err(|e| anyhow::anyhow!("insert provider_rate_limit_windows failed: {e}"))?;
    Ok(())
}

pub fn get_provider_rate_limit_window(
    db: &DbInstance,
    window_id: &str,
) -> Result<Option<ProviderRateLimitWindowRecord>> {
    let mut params = BTreeMap::new();
    params.insert("wid".into(), DataValue::from(window_id));
    let result = db
        .run_script(
            rate_limit_query("window_id = $wid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get provider_rate_limit_window failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_rate_limit(row)))
}

pub fn list_provider_rate_limit_windows(
    db: &DbInstance,
    provider_id: &str,
) -> Result<Vec<ProviderRateLimitWindowRecord>> {
    let mut params = BTreeMap::new();
    params.insert("provider".into(), DataValue::from(provider_id));
    let result = db
        .run_script(
            rate_limit_query("provider_id = $provider"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list provider_rate_limit_windows failed: {e}"))?;
    let mut windows: Vec<_> = result
        .rows
        .iter()
        .map(|row| row_to_rate_limit(row))
        .collect();
    windows.sort_by(|a, b| b.observed_at.cmp(&a.observed_at));
    Ok(windows)
}

fn rate_limit_put_script() -> &'static str {
    "?[window_id, provider_id, profile_id, model_id, limit_id, limit_name, \
     window_kind, used_percent, resets_at, raw_redacted_json, observed_at] \
     <- [[$wid, $provider, $profile, $model, $limit_id, $limit_name, \
     $kind, $used, $resets, $raw, $observed]] \
     :put provider_rate_limit_windows { window_id => provider_id, \
     profile_id, model_id, limit_id, limit_name, window_kind, \
     used_percent, resets_at, raw_redacted_json, observed_at }"
}

fn rate_limit_query(predicate: &'static str) -> &'static str {
    match predicate {
        "window_id = $wid" => {
            "?[window_id, provider_id, profile_id, model_id, limit_id, \
             limit_name, window_kind, used_percent, resets_at, \
             raw_redacted_json, observed_at] := *provider_rate_limit_windows{ \
             window_id, provider_id, profile_id, model_id, limit_id, \
             limit_name, window_kind, used_percent, resets_at, \
             raw_redacted_json, observed_at}, window_id = $wid"
        }
        _ => {
            "?[window_id, provider_id, profile_id, model_id, limit_id, \
             limit_name, window_kind, used_percent, resets_at, \
             raw_redacted_json, observed_at] := *provider_rate_limit_windows{ \
             window_id, provider_id, profile_id, model_id, limit_id, \
             limit_name, window_kind, used_percent, resets_at, \
             raw_redacted_json, observed_at}, provider_id = $provider"
        }
    }
}

fn row_to_rate_limit(row: &[DataValue]) -> ProviderRateLimitWindowRecord {
    ProviderRateLimitWindowRecord {
        window_id: str_col(row, 0).to_string(),
        provider_id: str_col(row, 1).to_string(),
        profile_id: non_empty(str_col(row, 2)),
        model_id: non_empty(str_col(row, 3)),
        limit_id: non_empty(str_col(row, 4)),
        limit_name: non_empty(str_col(row, 5)),
        window_kind: str_col(row, 6).to_string(),
        used_percent: parse_optional_percent(row[7].get_float().unwrap_or(-1.0)),
        resets_at: non_empty(str_col(row, 8)),
        raw_redacted_json: serde_json::from_str(str_col(row, 9))
            .unwrap_or_else(|_| serde_json::json!({})),
        observed_at: str_col(row, 10).to_string(),
    }
}

fn str_col(row: &[DataValue], index: usize) -> &str {
    row[index].get_str().unwrap_or("")
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn optional_percent(value: Option<f64>) -> f64 {
    value.map(|value| value.clamp(0.0, 100.0)).unwrap_or(-1.0)
}

fn parse_optional_percent(value: f64) -> Option<f64> {
    (value >= 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-provider-rate-limits-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn provider_rate_limit_window_roundtrips() {
        let db = test_db();
        let window = ProviderRateLimitWindowRecord::new(
            "limit-window-1",
            "openai-codex",
            "usage",
            "2026-05-08T12:00:00Z",
        )
        .with_profile("codex-oauth")
        .with_model("gpt-5.3-codex")
        .with_limit("weekly", "Weekly usage")
        .with_used_percent(101.0)
        .with_resets_at("2026-05-15T00:00:00Z")
        .with_redacted_json(serde_json::json!({"source": "headers"}));

        insert_provider_rate_limit_window(&db, &window).unwrap();
        let restored = get_provider_rate_limit_window(&db, "limit-window-1")
            .unwrap()
            .unwrap();

        assert_eq!(restored.provider_id, "openai-codex");
        assert_eq!(restored.profile_id.as_deref(), Some("codex-oauth"));
        assert_eq!(restored.used_percent, Some(100.0));
        assert!(restored.is_exhausted());
        assert_eq!(restored.raw_redacted_json["source"], "headers");
    }

    #[test]
    fn provider_rate_limit_windows_list_by_provider() {
        let db = test_db();
        insert_provider_rate_limit_window(
            &db,
            &ProviderRateLimitWindowRecord::new(
                "limit-window-1",
                "anthropic",
                "requests",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();
        insert_provider_rate_limit_window(
            &db,
            &ProviderRateLimitWindowRecord::new(
                "limit-window-2",
                "anthropic",
                "tokens",
                "2026-05-08T12:01:00Z",
            ),
        )
        .unwrap();

        let windows = list_provider_rate_limit_windows(&db, "anthropic").unwrap();

        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].window_id, "limit-window-2");
    }
}
