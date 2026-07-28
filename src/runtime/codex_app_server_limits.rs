//! Codex app-server rate-limit notification persistence.

use std::collections::HashSet;

use super::learning_store;
use archon_learning::provider_rate_limits::ProviderRateLimitWindowRecord;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

const PROVIDER_ID: &str = "openai-codex";
const WINDOW_KEYS: [&str; 2] = ["primary", "secondary"];

pub(crate) async fn record_rate_limits(params: &Value, model_id: Option<&str>) {
    let result = record_rate_limits_with(
        params.clone(),
        model_id.map(str::to_owned),
        |params, model_id| record_rate_limits_blocking(&params, model_id.as_deref()),
    )
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, provider = PROVIDER_ID, "Codex app-server rate limit recorder task failed");
    }
}

async fn record_rate_limits_with(
    params: Value,
    model_id: Option<String>,
    record: impl FnOnce(Value, Option<String>) + Send + 'static,
) -> Result<(), tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || record(params, model_id)).await
}

fn record_rate_limits_blocking(params: &Value, model_id: Option<&str>) {
    if let Err(error) =
        record_rate_limits_with_store(params, model_id, learning_store::acquire_default)
    {
        tracing::warn!(%error, provider = PROVIDER_ID, "Codex app-server rate limit persistence failed");
    }
}

fn record_rate_limits_with_store(
    params: &Value,
    model_id: Option<&str>,
    acquire_store: impl FnOnce() -> anyhow::Result<std::sync::Arc<cozo::DbInstance>>,
) -> anyhow::Result<()> {
    let windows = build_rate_limit_windows(params, model_id, Utc::now());
    if windows.is_empty() {
        return Ok(());
    }
    let db = acquire_store()?;
    for window in windows {
        archon_learning::provider_rate_limits::insert_provider_rate_limit_window(&db, &window)?;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn repeated_recorder_events_share_one_cached_open_and_schema_ensure() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("learning-state.db");
        super::learning_store::clear_for_tests(&path);
        let opens = Arc::new(AtomicUsize::new(0));
        let ensures = Arc::new(AtomicUsize::new(0));
        let first = serde_json::json!({
            "limitId": "codex",
            "limitName": "Codex",
            "primary": {"usedPercent": 100.0, "resetsAt": 1770000000}
        });
        let second = serde_json::json!({
            "limitId": "codex",
            "limitName": "Codex",
            "primary": {"usedPercent": 90.0, "resetsAt": 1770003600}
        });

        record_rate_limits_with_store(&first, Some("gpt-5.4"), {
            let path = path.clone();
            let opens = Arc::clone(&opens);
            let ensures = Arc::clone(&ensures);
            move || cached_test_store(&path, opens, ensures)
        })?;
        record_rate_limits_with_store(&second, Some("gpt-5.4"), {
            let path = path.clone();
            let opens = Arc::clone(&opens);
            let ensures = Arc::clone(&ensures);
            move || cached_test_store(&path, opens, ensures)
        })?;

        let db = super::learning_store::acquire_for_path(&path)?;
        let windows = archon_learning::provider_rate_limits::list_provider_rate_limit_windows(
            &db,
            PROVIDER_ID,
        )?;
        assert_eq!(opens.load(Ordering::SeqCst), 1);
        assert_eq!(ensures.load(Ordering::SeqCst), 1);
        assert_eq!(windows.len(), 2, "read back both persisted recorder events");
        assert_eq!(
            windows[0].raw_redacted_json["source"],
            "codex_app_server_notification"
        );
        super::learning_store::clear_for_tests(&path);
        Ok(())
    }

    fn cached_test_store(
        path: &Path,
        opens: Arc<AtomicUsize>,
        ensures: Arc<AtomicUsize>,
    ) -> anyhow::Result<Arc<cozo::DbInstance>> {
        super::learning_store::acquire_for_path_with(path, move |path| {
            opens.fetch_add(1, Ordering::SeqCst);
            let db = archon_learning::cozo_guard::open_sqlite_guarded(
                path.to_str().unwrap(),
                "open Codex rate-limit test store",
            )?;
            ensures.fetch_add(1, Ordering::SeqCst);
            archon_learning::schema::ensure_learning_schema(&db)?;
            Ok(db)
        })
    }

    #[tokio::test(flavor = "current_thread")]
    async fn notification_first_acquisition_runs_off_the_async_executor() {
        let notification = serde_json::json!({
            "limitId": "codex",
            "primary": {"usedPercent": 100.0, "resetsAt": 1770000000}
        });
        let executor_thread = std::thread::current().id();
        let (thread_tx, thread_rx) = tokio::sync::oneshot::channel();

        let work = tokio::spawn(record_rate_limits_with(
            notification,
            Some("gpt-5.4".to_string()),
            move |_, _| {
                thread_tx
                    .send(std::thread::current().id())
                    .expect("report recorder thread");
            },
        ));

        let recorder_thread = thread_rx.await.expect("recorder runs");
        work.await
            .expect("recorder future joins")
            .expect("blocking task joins");
        assert_ne!(recorder_thread, executor_thread);
    }

    #[tokio::test]
    async fn recorder_surfaces_a_panicking_worker_as_a_join_error() {
        let result = record_rate_limits_with(Value::Null, None, |_, _| {
            panic!("injected recorder panic");
        })
        .await;

        assert!(
            result
                .expect_err("panic must surface through join")
                .is_panic()
        );
    }

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
