use std::{
    fs,
    path::{Path, PathBuf},
};

use archon_learning::runtime_models::ProviderRuntimeEventRecord;

use super::*;

#[test]
fn missing_event_ledger_is_empty() {
    let events = read_recent_lines("web", Path::new("/not/real/events.jsonl"), 4);
    assert!(events.is_empty());
}

#[test]
fn provider_metrics_aggregate_usage_and_errors() {
    let first = ProviderRuntimeEventRecord::new(
        "event-1",
        "anthropic",
        "direct",
        "request_succeeded",
        "info",
        "now",
    )
    .with_model("claude")
    .with_redacted_json(serde_json::json!({
        "usage": { "input_count": 100, "output_count": 40 },
        "latency_ms": 250,
        "cost_usd": 0.002
    }));
    let second = ProviderRuntimeEventRecord::new(
        "event-2",
        "anthropic",
        "direct",
        "request_failed",
        "warn",
        "later",
    )
    .with_retry_count(2)
    .with_redacted_json(serde_json::json!({
        "usage": { "input_count": 20, "output_count": 0 },
        "latency_ms": 800
    }));
    let metrics = provider_metrics(&[first, second]);
    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].request_count, 2);
    assert_eq!(metrics[0].error_count, 1);
    assert_eq!(metrics[0].retry_count, 2);
    assert_eq!(metrics[0].input_tokens, 120);
    assert_eq!(metrics[0].output_tokens, 40);
    assert_eq!(metrics[0].latency_ms_p95, 800);
}

#[test]
fn provider_records_scan_session_store_even_when_local_db_is_empty() {
    let root = unique_temp_root();
    let cwd = root.join("project");
    let archon_home = root.join("home");
    let archon_data = root.join("data");
    fs::create_dir_all(cwd.join(".archon")).unwrap();
    fs::create_dir_all(&archon_data).unwrap();

    let empty_path = cwd.join(".archon/learning.db");
    let empty_db = open_test_learning_db(&empty_path);
    archon_learning::schema::ensure_learning_schema(&empty_db).unwrap();

    let data_path = archon_data.join("sessions/learning.db");
    let data_db = open_test_learning_db(&data_path);
    archon_learning::schema::ensure_learning_schema(&data_db).unwrap();
    archon_learning::runtime_events::insert_provider_runtime_event(
        &data_db,
        &ProviderRuntimeEventRecord::new(
            "event-from-data-dir",
            "codex",
            "oauth",
            "request_succeeded",
            "info",
            "2026-05-13T12:00:00Z",
        )
        .with_model("gpt"),
    )
    .unwrap();

    let paths = WebRuntimePaths {
        cwd: cwd.clone(),
        archon_home: archon_home.clone(),
        archon_data: archon_data.clone(),
        memory_db: archon_data.join("memory.db"),
        session_db: archon_data.join("sessions/sessions.db"),
        session_activity_root: archon_home.join("sessions"),
        world_model_root: archon_home.join("world-model"),
        reasoning_quality_root: archon_home.join("reasoning-quality"),
    };
    let records = provider_runtime_records(&cwd, &paths);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event_id, "event-from-data-dir");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_ledger_is_reported_as_missing_not_quiet() {
    let value = ledger("missing ledger", PathBuf::from("/not/real/events.jsonl"));
    assert_eq!(value.value, "0");
    assert_eq!(value.unit, "rows");
    assert_eq!(value.status, "missing");
}

fn unique_temp_root() -> PathBuf {
    std::env::temp_dir().join(format!("archon-web-metrics-{}", uuid::Uuid::new_v4()))
}

fn open_test_learning_db(path: &Path) -> cozo::DbInstance {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let path_str = path.to_string_lossy().to_string();
    cozo::DbInstance::new("sqlite", &path_str, "").unwrap()
}
