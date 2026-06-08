//! Codex app-server rate-limit notification persistence.

use std::collections::HashSet;

use archon_learning::provider_rate_limits::ProviderRateLimitWindowRecord;
use chrono::{DateTime, TimeZone, Utc};
use cozo::DbInstance;
use serde_json::Value;
use sha2::{Digest, Sha256};

const PROVIDER_ID: &str = "openai-codex";
const WINDOW_KEYS: [&str; 2] = ["primary", "secondary"];

pub(crate) fn record_rate_limits(params: &Value, model_id: Option<&str>) {
    let observed_at = Utc::now();
    let windows = build_rate_limit_windows(params, model_id, observed_at);
    if windows.is_empty() {
        return;
    }
    let Ok(db) = open_learning_db() else {
        return;
    };
    for window in windows {
        if let Err(error) =
            archon_learning::provider_rate_limits::insert_provider_rate_limit_window(&db, &window)
        {
            tracing::warn!(%error, provider = PROVIDER_ID, "Codex app-server rate limit persistence failed");
        }
    }
}

fn build_rate_limit_windows(
    params: &Value,
    model_id: Option<&str>,
    observed_at: DateTime<Utc>,
) -> Vec<ProviderRateLimitWindowRecord> {
    let mut snapshots = Vec::new();
    let mut seen = HashSet::new();
    collect_snapshots(params, &mut snapshots, &mut seen);
    snapshots
        .into_iter()
        .flat_map(|snapshot| records_for_snapshot(snapshot, model_id, observed_at))
        .collect()
}

fn collect_snapshots<'a>(
    value: &'a Value,
    snapshots: &mut Vec<&'a Value>,
    seen: &mut HashSet<String>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_snapshots(item, snapshots, seen);
            }
        }
        Value::Object(_) if is_snapshot(value) => {
            let signature = snapshot_signature(value);
            if seen.insert(signature) {
                snapshots.push(value);
            }
        }
        Value::Object(map) => {
            if let Some(by_limit_id) = map.get("rateLimitsByLimitId").and_then(Value::as_object) {
                for child in by_limit_id.values() {
                    collect_snapshots(child, snapshots, seen);
                }
            }
            for key in ["rateLimits", "data", "items"] {
                if let Some(child) = map.get(key) {
                    collect_snapshots(child, snapshots, seen);
                }
            }
        }
        _ => {}
    }
}

fn is_snapshot(value: &Value) -> bool {
    value.get("primary").and_then(Value::as_object).is_some()
        || value.get("secondary").and_then(Value::as_object).is_some()
        || value.get("rateLimitReachedType").is_some()
        || value.get("limitId").is_some()
        || value.get("limitName").is_some()
}

fn records_for_snapshot(
    snapshot: &Value,
    model_id: Option<&str>,
    observed_at: DateTime<Utc>,
) -> Vec<ProviderRateLimitWindowRecord> {
    let mut records = Vec::new();
    for key in WINDOW_KEYS {
        if let Some(window) = snapshot.get(key).filter(|value| value.is_object()) {
            records.push(record_for_window(
                snapshot,
                key,
                window,
                model_id,
                observed_at,
            ));
        }
    }
    if records.is_empty() {
        records.push(record_for_window(
            snapshot,
            "snapshot",
            &Value::Null,
            model_id,
            observed_at,
        ));
    }
    records
}

fn record_for_window(
    snapshot: &Value,
    window_key: &str,
    window: &Value,
    model_id: Option<&str>,
    observed_at: DateTime<Utc>,
) -> ProviderRateLimitWindowRecord {
    let limit_id = read_string(snapshot, "limitId").unwrap_or(PROVIDER_ID);
    let limit_name = read_string(snapshot, "limitName").unwrap_or("Codex usage");
    let used_percent = read_number(window, "usedPercent");
    let reached_type = read_string(snapshot, "rateLimitReachedType");
    let mut record = ProviderRateLimitWindowRecord::new(
        window_id(snapshot, window_key, observed_at),
        PROVIDER_ID,
        window_kind(reached_type, used_percent),
        observed_at.to_rfc3339(),
    )
    .with_limit(limit_id, limit_name)
    .with_redacted_json(redacted_payload(
        snapshot,
        window_key,
        window,
        reached_type,
        used_percent,
    ));
    if let Some(model_id) = model_id.filter(|value| !value.trim().is_empty()) {
        record = record.with_model(model_id.to_string());
    }
    if let Some(used_percent) = used_percent {
        record = record.with_used_percent(used_percent);
    }
    if let Some(resets_at) = read_resets_at(window) {
        record = record.with_resets_at(resets_at.to_rfc3339());
    }
    record
}

fn read_resets_at(window: &Value) -> Option<DateTime<Utc>> {
    let seconds = read_number(window, "resetsAt")?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Utc.timestamp_opt(seconds.trunc() as i64, 0).single()
}

fn window_kind(reached_type: Option<&str>, used_percent: Option<f64>) -> &'static str {
    if reached_type
        .map(|value| value.to_ascii_lowercase().contains("usage"))
        .unwrap_or(false)
        || used_percent.map(|value| value >= 100.0).unwrap_or(false)
    {
        "usage_limit"
    } else {
        "rate_limit"
    }
}

fn redacted_payload(
    snapshot: &Value,
    window_key: &str,
    window: &Value,
    reached_type: Option<&str>,
    used_percent: Option<f64>,
) -> Value {
    serde_json::json!({
        "source": "codex_app_server_notification",
        "window": window_key,
        "limit_id": read_string(snapshot, "limitId"),
        "limit_name": read_string(snapshot, "limitName"),
        "rate_limit_reached_type": reached_type,
        "used_percent": used_percent,
        "has_reset": read_resets_at(window).is_some(),
    })
}

fn window_id(snapshot: &Value, window_key: &str, observed_at: DateTime<Utc>) -> String {
    let signature = format!(
        "{}|{}|{}|{}",
        snapshot_signature(snapshot),
        window_key,
        read_number(snapshot.get(window_key).unwrap_or(&Value::Null), "resetsAt").unwrap_or(0.0),
        observed_at.timestamp()
    );
    format!("codex-app-limit-{}", hex::encode(Sha256::digest(signature)))
}

fn snapshot_signature(snapshot: &Value) -> String {
    format!(
        "{}|{}|{}|{}",
        read_string(snapshot, "limitId").unwrap_or(""),
        read_string(snapshot, "limitName").unwrap_or(""),
        window_signature(snapshot.get("primary")),
        window_signature(snapshot.get("secondary"))
    )
}

fn window_signature(window: Option<&Value>) -> String {
    let Some(window) = window else {
        return String::new();
    };
    format!(
        "{}:{}",
        read_number(window, "usedPercent").unwrap_or(-1.0),
        read_number(window, "resetsAt").unwrap_or(0.0)
    )
}

fn read_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
}

fn read_number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn open_learning_db() -> anyhow::Result<DbInstance> {
    let path = crate::command::store_paths::learning_db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_str = path.to_string_lossy().to_string();
    let db = archon_learning::cozo_guard::open_sqlite_guarded(&path_str, "open learning db")?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_nested_codex_rate_limit_windows() {
        let observed = DateTime::parse_from_rfc3339("2026-05-09T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let params = serde_json::json!({
            "rateLimitsByLimitId": {
                "codex": {
                    "limitId": "codex",
                    "limitName": "Codex",
                    "rateLimitReachedType": "usage_limit",
                    "primary": {"usedPercent": 100.0, "resetsAt": 1770000000},
                    "secondary": {"usedPercent": 40.0, "resetsAt": 1770003600}
                }
            }
        });

        let windows = build_rate_limit_windows(&params, Some("gpt-5.4"), observed);

        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].provider_id, "openai-codex");
        assert_eq!(windows[0].window_kind, "usage_limit");
        assert_eq!(windows[0].used_percent, Some(100.0));
        assert_eq!(windows[0].model_id.as_deref(), Some("gpt-5.4"));
        assert!(windows[0].resets_at.is_some());
        assert_eq!(
            windows[0].raw_redacted_json["source"],
            "codex_app_server_notification"
        );
    }

    #[test]
    fn ignores_payloads_without_rate_limit_snapshots() {
        let windows = build_rate_limit_windows(
            &serde_json::json!({"threadId": "thread-1"}),
            None,
            Utc::now(),
        );

        assert!(windows.is_empty());
    }
}
