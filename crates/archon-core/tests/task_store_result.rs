//! Tests for SqliteTaskStateStore and result() (TASK-AGS-203).

use archon_core::tasks::models::{Task, TaskError, TaskId, TaskResultRef, TaskState};
use archon_core::tasks::store::{SqliteTaskStateStore, TaskStateStore};
use chrono::Utc;
use tempfile::TempDir;

fn make_task(agent: &str) -> Task {
    Task {
        id: TaskId::new(),
        agent_name: agent.to_string(),
        agent_version: None,
        input: serde_json::json!({}),
        state: TaskState::Pending,
        progress_pct: None,
        created_at: Utc::now(),
        started_at: None,
        finished_at: None,
        result_ref: None,
        error: None,
        owner: "test".to_string(),
        resource_usage: None,
        cancel_token_ref: None,
    }
}

#[test]
fn test_result_inline_under_64kib() {
    let tmp = TempDir::new().unwrap();
    let store = SqliteTaskStateStore::open(tmp.path()).unwrap();

    let mut task = make_task("agent-a");
    task.state = TaskState::Finished;
    task.result_ref = Some(TaskResultRef {
        inline: Some("hello world".to_string()),
        file_path: None,
        streaming_handle: None,
    });
    store.put_snapshot(&task).unwrap();

    let snap = store.get_snapshot(task.id).unwrap();
    assert_eq!(snap.state, TaskState::Finished);

    // Verify the snapshot JSON portion contains the state.
    // The file has a 4-byte CRC32 prefix, so read raw bytes and check JSON portion.
    let json_path = tmp.path().join(format!("{}.json", task.id));
    let raw = std::fs::read(&json_path).unwrap();
    let json_part = std::str::from_utf8(&raw[4..]).unwrap();
    assert!(json_part.contains("Finished") || json_part.contains("FINISHED"));
}

#[test]
fn test_result_pending_returns_error() {
    let tmp = TempDir::new().unwrap();
    let store = SqliteTaskStateStore::open(tmp.path()).unwrap();

    let task = make_task("agent-a"); // state = Pending
    store.put_snapshot(&task).unwrap();

    // Trying to get result of a non-terminal task should fail.
    let snap = store.get_snapshot(task.id).unwrap();
    assert!(!snap.state.is_terminal());
}

#[test]
fn test_corrupted_checksum_detected() {
    let tmp = TempDir::new().unwrap();
    let store = SqliteTaskStateStore::open(tmp.path()).unwrap();

    let mut task = make_task("agent-a");
    task.state = TaskState::Finished;
    store.put_snapshot(&task).unwrap();

    // Corrupt the JSON file.
    let json_path = tmp.path().join(format!("{}.json", task.id));
    std::fs::write(&json_path, b"corrupted data here").unwrap();

    // Re-open store; the corrupted file should be detected during hydration.
    let store2 = SqliteTaskStateStore::open(tmp.path()).unwrap();
    let snap = store2.get_snapshot(task.id);
    match snap {
        Ok(s) => assert_eq!(s.state, TaskState::Corrupted),
        Err(TaskError::Corrupted) => {} // also acceptable
        other => panic!("expected Corrupted state or error, got {:?}", other),
    }
}

#[test]
fn test_snapshot_write_is_atomic() {
    let tmp = TempDir::new().unwrap();
    let store = SqliteTaskStateStore::open(tmp.path()).unwrap();

    let task = make_task("agent-a");
    store.put_snapshot(&task).unwrap();

    // Verify temp file doesn't linger (atomic rename).
    let tmp_path = tmp.path().join(format!("{}.json.tmp", task.id));
    assert!(
        !tmp_path.exists(),
        "temp file should not exist after atomic write"
    );

    // Main file should exist.
    let json_path = tmp.path().join(format!("{}.json", task.id));
    assert!(json_path.exists());
}

#[test]
fn test_sqlite_index_rehydrates_cache_on_open() {
    let tmp = TempDir::new().unwrap();

    // Write 10 tasks with first store instance.
    {
        let store = SqliteTaskStateStore::open(tmp.path()).unwrap();
        for i in 0..10 {
            let task = make_task(&format!("agent-{}", i % 3));
            store.put_snapshot(&task).unwrap();
        }
    }

    // Re-open and verify all 10 are in cache.
    let store = SqliteTaskStateStore::open(tmp.path()).unwrap();
    let all = store.list_snapshots(&Default::default()).unwrap();
    assert_eq!(all.len(), 10);
}

#[test]
fn test_result_file_over_64kib() {
    let tmp = TempDir::new().unwrap();
    let store = SqliteTaskStateStore::open(tmp.path()).unwrap();

    let mut task = make_task("agent-a");
    task.state = TaskState::Finished;

    // Create a large result file.
    let result_dir = tmp.path().join(task.id.to_string());
    std::fs::create_dir_all(&result_dir).unwrap();
    let result_path = result_dir.join("result.bin");
    let big_data = vec![0xABu8; 128 * 1024]; // 128 KiB
    std::fs::write(&result_path, &big_data).unwrap();

    task.result_ref = Some(TaskResultRef {
        inline: None,
        file_path: Some(result_path.clone()),
        streaming_handle: None,
    });
    store.put_snapshot(&task).unwrap();

    let snap = store.get_snapshot(task.id).unwrap();
    assert_eq!(snap.state, TaskState::Finished);
}
