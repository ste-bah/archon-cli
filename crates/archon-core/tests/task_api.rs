//! Tests for CliTaskApi adapter (TASK-AGS-208).

use std::sync::Arc;

use archon_core::agents::registry::AgentRegistry;
use archon_core::tasks::api::{parse_duration_ago, CliTaskApi};
use archon_core::tasks::metrics::MetricsRegistry;
use archon_core::tasks::models::TaskError;
use archon_core::tasks::service::{DefaultTaskService, TaskService};
use tempfile::TempDir;

fn make_api() -> CliTaskApi {
    let tmp = TempDir::new().unwrap();
    let registry = Arc::new(AgentRegistry::load(tmp.path()));
    let service: Arc<dyn TaskService> =
        Arc::new(DefaultTaskService::new(registry, 10000));
    let metrics = Arc::new(MetricsRegistry::new());
    CliTaskApi::new(service, metrics)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_api_submit_returns_task_id() {
    let api = make_api();
    let result = api
        .submit(
            "general-purpose".to_string(),
            None,
            None,
            false,
        )
        .await;
    assert!(result.is_ok(), "submit should succeed: {:?}", result.err());
    let json_str = result.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert!(
        parsed.get("task_id").is_some(),
        "response should contain task_id field"
    );
    let task_id_str = parsed["task_id"].as_str().unwrap();
    // Validate it's a UUID
    assert!(
        uuid::Uuid::parse_str(task_id_str).is_ok(),
        "task_id should be a valid UUID"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_api_status_returns_snapshot() {
    let api = make_api();
    let submit_result = api
        .submit("general-purpose".to_string(), None, None, false)
        .await
        .unwrap();
    let submit_json: serde_json::Value = serde_json::from_str(&submit_result).unwrap();
    let task_id = submit_json["task_id"].as_str().unwrap();

    let status_result = api.status(task_id, false).await;
    assert!(
        status_result.is_ok(),
        "status should succeed: {:?}",
        status_result.err()
    );
    let status_json: serde_json::Value =
        serde_json::from_str(&status_result.unwrap()).unwrap();
    assert!(
        status_json.get("state").is_some(),
        "status response should contain state field"
    );
    assert!(
        status_json.get("id").is_some(),
        "status response should contain id field"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_api_list_returns_array() {
    let api = make_api();

    // Submit 3 tasks
    for _ in 0..3 {
        api.submit("general-purpose".to_string(), None, None, false)
            .await
            .unwrap();
    }

    let list_result = api.list(None, None, None).await;
    assert!(
        list_result.is_ok(),
        "list should succeed: {:?}",
        list_result.err()
    );
    let list_json: serde_json::Value =
        serde_json::from_str(&list_result.unwrap()).unwrap();
    assert!(list_json.is_array(), "list response should be a JSON array");
    assert_eq!(
        list_json.as_array().unwrap().len(),
        3,
        "list should return 3 tasks"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_api_result_pending_returns_error() {
    let api = make_api();
    let submit_result = api
        .submit("general-purpose".to_string(), None, None, false)
        .await
        .unwrap();
    let submit_json: serde_json::Value = serde_json::from_str(&submit_result).unwrap();
    let task_id = submit_json["task_id"].as_str().unwrap();

    let result = api.result(task_id, false).await;
    assert!(result.is_err(), "result on pending task should error");
    match result.unwrap_err() {
        TaskError::Pending => {} // expected
        other => panic!("expected Pending error, got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_api_cancel_returns_cancelled() {
    let api = make_api();
    let submit_result = api
        .submit("general-purpose".to_string(), None, None, false)
        .await
        .unwrap();
    let submit_json: serde_json::Value = serde_json::from_str(&submit_result).unwrap();
    let task_id = submit_json["task_id"].as_str().unwrap();

    let cancel_result = api.cancel(task_id).await;
    assert!(
        cancel_result.is_ok(),
        "cancel should succeed: {:?}",
        cancel_result.err()
    );
    let cancel_json: serde_json::Value =
        serde_json::from_str(&cancel_result.unwrap()).unwrap();
    assert_eq!(
        cancel_json["status"].as_str().unwrap(),
        "cancelled",
        "cancel response status should be 'cancelled'"
    );
    assert_eq!(
        cancel_json["task_id"].as_str().unwrap(),
        task_id,
        "cancel response should echo task_id"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_api_metrics_prometheus_format() {
    let api = make_api();
    let output = api.metrics();
    assert!(
        output.contains("tasks_started_total"),
        "metrics should contain tasks_started_total"
    );
    assert!(
        output.contains("tasks_finished_total"),
        "metrics should contain tasks_finished_total"
    );
    assert!(
        output.contains("tasks_failed_total"),
        "metrics should contain tasks_failed_total"
    );
    assert!(
        output.contains("tasks_cancelled_total"),
        "metrics should contain tasks_cancelled_total"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_api_parse_duration_ago() {
    let now = chrono::Utc::now();

    // 1h ago
    let one_hour = parse_duration_ago("1h");
    assert!(one_hour.is_some());
    let diff = now - one_hour.unwrap();
    // Allow 2 seconds of drift for test execution
    assert!(
        (diff.num_seconds() - 3600).abs() < 2,
        "1h should be ~3600s ago, got {}s",
        diff.num_seconds()
    );

    // 30m ago
    let thirty_min = parse_duration_ago("30m");
    assert!(thirty_min.is_some());
    let diff = now - thirty_min.unwrap();
    assert!(
        (diff.num_seconds() - 1800).abs() < 2,
        "30m should be ~1800s ago, got {}s",
        diff.num_seconds()
    );

    // 7d ago
    let seven_days = parse_duration_ago("7d");
    assert!(seven_days.is_some());
    let diff = now - seven_days.unwrap();
    assert!(
        (diff.num_seconds() - 604800).abs() < 2,
        "7d should be ~604800s ago, got {}s",
        diff.num_seconds()
    );

    // Invalid
    assert!(parse_duration_ago("").is_none());
    assert!(parse_duration_ago("abc").is_none());
    assert!(parse_duration_ago("x5").is_none());
}
