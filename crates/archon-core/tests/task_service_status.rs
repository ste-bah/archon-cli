//! Tests for TaskService::status() and list() (TASK-AGS-202).

use archon_core::agents::registry::AgentRegistry;
use archon_core::tasks::models::{SubmitRequest, TaskError, TaskFilter, TaskState};
use archon_core::tasks::service::{DefaultTaskService, TaskService};
use std::sync::Arc;
use tempfile::TempDir;

fn make_service() -> DefaultTaskService {
    let tmp = TempDir::new().unwrap();
    let registry = Arc::new(AgentRegistry::load(tmp.path()));
    DefaultTaskService::new(registry, 1000)
}

fn make_request(agent: &str) -> SubmitRequest {
    SubmitRequest {
        agent_name: agent.to_string(),
        agent_version: None,
        input: serde_json::json!({}),
        owner: "test".to_string(),
    }
}

#[tokio::test]
async fn test_status_returns_current_state() {
    let svc = make_service();
    let id = svc.submit(make_request("general-purpose")).await.unwrap();
    let snap = svc.status(id).await.unwrap();
    assert_eq!(snap.id, id);
    assert_eq!(snap.state, TaskState::Pending);
    assert_eq!(snap.agent_name, "general-purpose");
}

#[tokio::test]
async fn test_status_unknown_id_returns_not_found() {
    let svc = make_service();
    let fake_id = archon_core::tasks::models::TaskId::new();
    let result = svc.status(fake_id).await;
    assert!(matches!(result, Err(TaskError::NotFound(_))));
}

#[tokio::test]
async fn test_status_p95_under_50ms() {
    let svc = make_service();
    let id = svc.submit(make_request("general-purpose")).await.unwrap();
    let mut times = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let start = std::time::Instant::now();
        let _ = svc.status(id).await.unwrap();
        times.push(start.elapsed());
    }
    times.sort();
    let p95 = times[949];
    assert!(p95.as_millis() < 50, "p95 was {}ms", p95.as_millis());
}

#[tokio::test]
async fn test_concurrent_status_polls_consistent() {
    let svc = Arc::new(make_service());
    let id = svc.submit(make_request("general-purpose")).await.unwrap();

    let mut handles = Vec::new();
    for _ in 0..100 {
        let svc = svc.clone();
        handles.push(tokio::spawn(async move {
            let snap = svc.status(id).await.unwrap();
            assert_eq!(snap.state, TaskState::Pending);
            snap
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
}

#[tokio::test]
async fn test_list_filter_by_state() {
    let svc = make_service();
    // Submit 3 tasks (all PENDING)
    for _ in 0..3 {
        svc.submit(make_request("general-purpose")).await.unwrap();
    }
    let filter = TaskFilter {
        state: Some(TaskState::Pending),
        ..Default::default()
    };
    let results = svc.list(filter).await.unwrap();
    assert_eq!(results.len(), 3);

    // Filter by RUNNING should return empty
    let filter = TaskFilter {
        state: Some(TaskState::Running),
        ..Default::default()
    };
    let results = svc.list(filter).await.unwrap();
    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn test_list_filter_by_agent() {
    let svc = make_service();
    svc.submit(make_request("general-purpose")).await.unwrap();
    svc.submit(make_request("explore")).await.unwrap();
    svc.submit(make_request("general-purpose")).await.unwrap();

    let filter = TaskFilter {
        agent_name: Some("general-purpose".to_string()),
        ..Default::default()
    };
    let results = svc.list(filter).await.unwrap();
    assert_eq!(results.len(), 2);
}
