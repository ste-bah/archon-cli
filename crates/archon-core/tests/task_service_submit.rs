//! Tests for TaskService::submit() (TASK-AGS-201).

use archon_core::agents::registry::AgentRegistry;
use archon_core::tasks::models::{SubmitRequest, TaskError};
use archon_core::tasks::service::{DefaultTaskService, TaskService};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

fn make_service(max_queue: usize) -> DefaultTaskService {
    let tmp = TempDir::new().unwrap();
    let registry = Arc::new(AgentRegistry::load(tmp.path()));
    DefaultTaskService::new(registry, max_queue)
}

fn make_request(agent_name: &str) -> SubmitRequest {
    SubmitRequest {
        agent_name: agent_name.to_string(),
        agent_version: None,
        input: serde_json::json!({"prompt": "test"}),
        owner: "test-user".to_string(),
    }
}

#[tokio::test]
async fn test_submit_returns_within_100ms() {
    let svc = make_service(1000);
    let mut times = Vec::with_capacity(100);
    for _ in 0..100 {
        let start = Instant::now();
        let _id = svc.submit(make_request("general-purpose")).await.unwrap();
        times.push(start.elapsed());
    }
    times.sort();
    let p95 = times[94];
    assert!(
        p95.as_millis() < 100,
        "p95 submit latency was {}ms, expected < 100ms",
        p95.as_millis()
    );
}

#[tokio::test]
async fn test_submit_returns_unique_ids() {
    let svc = make_service(2000);
    let mut ids = HashSet::new();
    for _ in 0..1000 {
        let id = svc.submit(make_request("general-purpose")).await.unwrap();
        ids.insert(id);
    }
    assert_eq!(ids.len(), 1000);
}

#[tokio::test]
async fn test_submit_queue_full_returns_error() {
    let svc = make_service(5);
    for _ in 0..5 {
        svc.submit(make_request("general-purpose")).await.unwrap();
    }
    let result = svc.submit(make_request("general-purpose")).await;
    assert!(
        matches!(result, Err(TaskError::QueueFull)),
        "expected QueueFull, got {:?}",
        result
    );
}

#[tokio::test]
async fn test_submit_unknown_agent_returns_error() {
    let svc = make_service(100);
    let result = svc.submit(make_request("nonexistent-agent-xyz")).await;
    match &result {
        Err(TaskError::NotFound(_)) => {}
        other => panic!("expected NotFound, got {:?}", other),
    }
}
