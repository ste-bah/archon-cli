//! Tests for the task models module (TASK-AGS-200).

use archon_core::tasks::models::{
    ResourceSample, SubmitRequest, Task, TaskError, TaskEvent, TaskEventKind, TaskId,
    TaskResultRef, TaskState,
};
use chrono::Utc;
use std::collections::HashSet;
use std::str::FromStr;

#[test]
fn test_task_state_has_six_variants() {
    let variants = [
        TaskState::Pending,
        TaskState::Running,
        TaskState::Finished,
        TaskState::Failed,
        TaskState::Cancelled,
        TaskState::Corrupted,
    ];
    // All six must be distinct.
    let set: HashSet<TaskState> = variants.iter().copied().collect();
    assert_eq!(set.len(), 6);

    // Terminal checks.
    assert!(!TaskState::Pending.is_terminal());
    assert!(!TaskState::Running.is_terminal());
    assert!(TaskState::Finished.is_terminal());
    assert!(TaskState::Failed.is_terminal());
    assert!(TaskState::Cancelled.is_terminal());
    assert!(TaskState::Corrupted.is_terminal());
}

#[test]
fn test_task_id_display_and_from_str() {
    let id = TaskId::new();
    let s = id.to_string();
    let parsed = TaskId::from_str(&s).expect("parse back");
    assert_eq!(id, parsed);
}

#[test]
fn test_task_id_unique() {
    let ids: HashSet<TaskId> = (0..1000).map(|_| TaskId::new()).collect();
    assert_eq!(ids.len(), 1000);
}

#[test]
fn test_task_event_seq_is_u64() {
    let evt = TaskEvent {
        task_id: TaskId::new(),
        seq: u64::MAX,
        kind: TaskEventKind::Started,
        payload: serde_json::json!({}),
        at: Utc::now(),
    };
    assert_eq!(evt.seq, u64::MAX);
}

#[test]
fn test_task_error_variants() {
    let id = TaskId::new();

    let errors: Vec<TaskError> = vec![
        TaskError::NotFound(id),
        TaskError::InvalidState,
        TaskError::Pending,
        TaskError::AlreadyCancelled,
        TaskError::Corrupted,
        TaskError::QueueFull,
        TaskError::Io(std::io::Error::new(std::io::ErrorKind::Other, "test")),
    ];

    // Each variant should produce a non-empty Display string.
    for e in &errors {
        let msg = format!("{e}");
        assert!(!msg.is_empty(), "error variant should have a message");
    }

    assert_eq!(errors.len(), 7);
}

#[test]
fn test_task_serde_roundtrip() {
    let task = Task {
        id: TaskId::new(),
        agent_name: "test-agent".to_string(),
        agent_version: Some(semver::Version::new(1, 2, 3)),
        input: serde_json::json!({"key": "value"}),
        state: TaskState::Pending,
        progress_pct: Some(42.0),
        created_at: Utc::now(),
        started_at: None,
        finished_at: None,
        result_ref: Some(TaskResultRef {
            inline: Some("hello".to_string()),
            file_path: None,
            streaming_handle: None,
        }),
        error: None,
        owner: "user-1".to_string(),
        resource_usage: Some(ResourceSample {
            cpu_ms: 100,
            rss_bytes: 2048,
        }),
        cancel_token_ref: None,
    };

    let json = serde_json::to_string(&task).expect("serialize");
    let back: Task = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(task, back);
}

#[test]
fn test_task_result_ref_variants() {
    // Inline variant.
    let inline = TaskResultRef {
        inline: Some("data".to_string()),
        file_path: None,
        streaming_handle: None,
    };
    assert!(inline.inline.is_some());
    assert!(inline.file_path.is_none());
    assert!(inline.streaming_handle.is_none());

    // File path variant.
    let file = TaskResultRef {
        inline: None,
        file_path: Some(std::path::PathBuf::from("/tmp/result.bin")),
        streaming_handle: None,
    };
    assert!(file.file_path.is_some());

    // Streaming handle variant.
    let stream = TaskResultRef {
        inline: None,
        file_path: None,
        streaming_handle: Some(42),
    };
    assert_eq!(stream.streaming_handle, Some(42));
}

#[test]
fn test_submit_request_fields() {
    let req = SubmitRequest {
        agent_name: "my-agent".to_string(),
        agent_version: Some(semver::Version::new(0, 1, 0)),
        input: serde_json::json!({"prompt": "hello"}),
        owner: "owner-abc".to_string(),
    };
    assert_eq!(req.agent_name, "my-agent");
    assert_eq!(req.owner, "owner-abc");
    assert!(req.agent_version.is_some());
    assert_eq!(req.input["prompt"], "hello");
}

#[test]
fn test_resource_sample_fields() {
    let sample = ResourceSample {
        cpu_ms: 1234,
        rss_bytes: 567890,
    };
    assert_eq!(sample.cpu_ms, 1234);
    assert_eq!(sample.rss_bytes, 567890);
}
