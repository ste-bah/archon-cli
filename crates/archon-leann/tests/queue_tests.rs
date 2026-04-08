//! Tests for the queue processor (TASK-PIPE-D05).
//!
//! Validates: empty queue, missing queue file, add_to_queue, queue_status,
//! processing success/failure, dead-letter escalation, and JSON format compatibility.

use std::path::{Path, PathBuf};

use archon_leann::indexer::{EmbeddingConfig, EmbeddingProviderKind, Indexer};
use archon_leann::queue::{QueueEntry, QueueProcessor};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_db() -> cozo::DbInstance {
    cozo::DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB")
}

fn test_indexer() -> Indexer {
    let db = test_db();
    let config = EmbeddingConfig {
        provider: EmbeddingProviderKind::Mock,
        dimension: 8,
    };
    let indexer = Indexer::new(db, config, None).expect("indexer creation");
    indexer.ensure_schema().expect("ensure_schema");
    indexer
}

/// Read queue JSON from a file and parse it.
fn read_queue_json(path: &Path) -> Vec<serde_json::Value> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    if content.trim().is_empty() {
        return vec![];
    }
    serde_json::from_str(&content).expect("valid JSON array")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 1. Process empty queue returns QueueResult with all zeros.
#[tokio::test]
async fn test_process_empty_queue_returns_zeros() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("queue.json");
    // Write an empty array
    std::fs::write(&queue_path, "[]").unwrap();

    let indexer = test_indexer();
    let processor = QueueProcessor::new(&indexer);
    let result = processor.process_queue(&queue_path).await.unwrap();

    assert_eq!(result.processed, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.remaining, 0);
}

/// 2. Process non-existent queue file returns QueueResult with all zeros.
#[tokio::test]
async fn test_process_nonexistent_queue_returns_zeros() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("does_not_exist.json");

    let indexer = test_indexer();
    let processor = QueueProcessor::new(&indexer);
    let result = processor.process_queue(&queue_path).await.unwrap();

    assert_eq!(result.processed, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.remaining, 0);
}

/// 3. add_to_queue creates queue file with correct JSON format.
#[test]
fn test_add_to_queue_creates_file_with_correct_format() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("queue.json");

    let files = vec![
        PathBuf::from("/some/file.rs"),
        PathBuf::from("/other/file.py"),
    ];
    QueueProcessor::add_to_queue(&queue_path, &files).unwrap();

    assert!(queue_path.exists(), "queue file should be created");

    let entries = read_queue_json(&queue_path);
    assert_eq!(entries.len(), 2, "should have 2 entries");

    // Verify camelCase field names
    let first = &entries[0];
    assert!(
        first.get("filePath").is_some(),
        "should have 'filePath' field (camelCase)"
    );
    assert!(
        first.get("addedAt").is_some(),
        "should have 'addedAt' field (camelCase)"
    );

    // Verify file_path field is NOT present (should be filePath)
    assert!(
        first.get("file_path").is_none(),
        "should NOT have snake_case 'file_path'"
    );

    // Verify the paths are correct
    let paths: Vec<String> = entries
        .iter()
        .map(|e| e["filePath"].as_str().unwrap().to_string())
        .collect();
    assert!(paths.contains(&"/some/file.rs".to_string()));
    assert!(paths.contains(&"/other/file.py".to_string()));
}

/// 4. add_to_queue appends to existing queue without losing prior entries.
#[test]
fn test_add_to_queue_appends_without_losing_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("queue.json");

    // First batch
    let batch1 = vec![PathBuf::from("/first/a.rs")];
    QueueProcessor::add_to_queue(&queue_path, &batch1).unwrap();

    // Second batch
    let batch2 = vec![PathBuf::from("/second/b.py"), PathBuf::from("/third/c.go")];
    QueueProcessor::add_to_queue(&queue_path, &batch2).unwrap();

    let entries = read_queue_json(&queue_path);
    assert_eq!(
        entries.len(),
        3,
        "should have 3 total entries after two appends"
    );

    let paths: Vec<String> = entries
        .iter()
        .map(|e| e["filePath"].as_str().unwrap().to_string())
        .collect();
    assert!(paths.contains(&"/first/a.rs".to_string()));
    assert!(paths.contains(&"/second/b.py".to_string()));
    assert!(paths.contains(&"/third/c.go".to_string()));
}

/// 5. queue_status returns correct count without modifying queue.
#[test]
fn test_queue_status_returns_count_without_modifying() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("queue.json");

    let files = vec![
        PathBuf::from("/a.rs"),
        PathBuf::from("/b.rs"),
        PathBuf::from("/c.rs"),
    ];
    QueueProcessor::add_to_queue(&queue_path, &files).unwrap();

    let before_mtime = std::fs::metadata(&queue_path).unwrap().modified().unwrap();

    let status = QueueProcessor::queue_status(&queue_path).unwrap();
    assert_eq!(status.pending, 3, "should report 3 pending files");
    assert_eq!(status.dead_letter, 0, "should have 0 dead-letter entries");
    assert_eq!(status.files.len(), 3, "files list should have 3 entries");

    // File should NOT be modified by queue_status
    let after_mtime = std::fs::metadata(&queue_path).unwrap().modified().unwrap();
    assert_eq!(
        before_mtime, after_mtime,
        "queue_status should not modify the file"
    );
}

/// 5b. queue_status on non-existent file returns zeros.
#[test]
fn test_queue_status_nonexistent_returns_zeros() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("missing.json");

    let status = QueueProcessor::queue_status(&queue_path).unwrap();
    assert_eq!(status.pending, 0);
    assert_eq!(status.dead_letter, 0);
    assert!(status.files.is_empty());
}

/// 6. Processing a queue of files: successful files removed, failed files (deleted) remain.
#[tokio::test]
async fn test_process_queue_success_and_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("queue.json");

    // Create a real source file that CAN be indexed
    let real_file = tmp.path().join("real.rs");
    std::fs::write(&real_file, "fn processable() -> i32 {\n    42\n}\n").unwrap();

    // A file that does NOT exist (will fail indexing)
    let missing_file = tmp.path().join("missing_does_not_exist.rs");

    let files = vec![real_file.clone(), missing_file.clone()];
    QueueProcessor::add_to_queue(&queue_path, &files).unwrap();

    let indexer = test_indexer();
    let processor = QueueProcessor::new(&indexer);
    let result = processor.process_queue(&queue_path).await.unwrap();

    // real_file should succeed
    assert_eq!(result.processed, 1, "expected 1 successful file");
    // missing_file should fail (1 error, below dead-letter threshold of 3)
    assert_eq!(result.failed, 1, "expected 1 failed file");

    // Remaining queue should have the failed entry (error_count=1), not the succeeded one
    let entries = read_queue_json(&queue_path);
    assert_eq!(entries.len(), 1, "only the failed entry should remain");

    let remaining_path = entries[0]["filePath"].as_str().unwrap();
    assert_eq!(
        remaining_path,
        missing_file.to_string_lossy().as_ref(),
        "remaining entry should be the failed file"
    );
    assert_eq!(
        entries[0]["errorCount"].as_u64().unwrap_or(0),
        1,
        "error_count should be 1 after first failure"
    );
}

/// 7. Dead-letter: after 3 failures for same file, file has error_count >= MAX_RETRIES
///    and queue_status reports it in dead_letter.
#[tokio::test]
async fn test_dead_letter_after_max_retries() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("queue.json");

    // A file that never exists (will always fail)
    let ghost_file = tmp.path().join("ghost_never_exists.rs");

    let files = vec![ghost_file.clone()];
    QueueProcessor::add_to_queue(&queue_path, &files).unwrap();

    let indexer = test_indexer();
    let processor = QueueProcessor::new(&indexer);

    // Process 3 times to exhaust retries
    for i in 1..=3 {
        let result = processor.process_queue(&queue_path).await.unwrap();
        // Should fail each time
        assert_eq!(result.failed, 1, "iteration {}: should fail", i);
        // File should still remain (it hasn't hit dead-letter threshold yet on iteration 3,
        // it has error_count = i which is <= MAX_RETRIES)
    }

    // After 3 failures: error_count should be 3 (= MAX_RETRIES), file in dead_letter
    let status = QueueProcessor::queue_status(&queue_path).unwrap();
    assert_eq!(
        status.dead_letter, 1,
        "file with error_count >= MAX_RETRIES should be in dead_letter"
    );
    assert_eq!(
        status.pending, 0,
        "dead-lettered file should not be in pending"
    );

    // 4th process: dead-lettered files are NOT processed again
    let result = processor.process_queue(&queue_path).await.unwrap();
    assert_eq!(
        result.processed, 0,
        "dead-lettered file should not be retried"
    );
    assert_eq!(
        result.failed, 0,
        "dead-lettered file should not count as new failure"
    );
}

/// 8. JSON format compatibility with `.archon/runtime/leann-index-queue.json`:
///    `[{"filePath": "/path", "addedAt": "2024-01-01T00:00:00Z"}]`
#[tokio::test]
async fn test_json_format_compatibility() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("queue.json");

    // Write queue in the exact format used by hooks (no errorCount field)
    let json_content =
        r#"[{"filePath": "/nonexistent/hook_added_file.rs", "addedAt": "2024-01-01T00:00:00Z"}]"#;
    std::fs::write(&queue_path, json_content).unwrap();

    let indexer = test_indexer();
    let processor = QueueProcessor::new(&indexer);

    // Should parse without error even though errorCount is missing
    let result = processor.process_queue(&queue_path).await.unwrap();

    // File doesn't exist so it will fail, but it should parse and attempt indexing
    // (processed = 0, failed = 1) - proves it was parsed and attempted
    let total_attempts = result.processed + result.failed;
    assert_eq!(
        total_attempts, 1,
        "should attempt to process the 1 entry from hook format"
    );
}

/// 9. add_to_queue uses atomic write (temp file + rename) — no partial writes.
///    We test this indirectly: result is always valid JSON.
#[test]
fn test_add_to_queue_produces_valid_json() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("queue.json");

    for i in 0..10 {
        let files = vec![PathBuf::from(format!("/file_{}.rs", i))];
        QueueProcessor::add_to_queue(&queue_path, &files).unwrap();
    }

    let content = std::fs::read_to_string(&queue_path).unwrap();
    let parsed: Result<Vec<QueueEntry>, _> = serde_json::from_str(&content);
    assert!(
        parsed.is_ok(),
        "queue file should always be valid JSON after multiple appends"
    );
    assert_eq!(parsed.unwrap().len(), 10);
}

/// 10. QueueResult.remaining matches entries left in queue file after processing.
#[tokio::test]
async fn test_queue_result_remaining_matches_file() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_path = tmp.path().join("queue.json");

    // 2 real files, 2 missing files
    let real1 = tmp.path().join("real1.rs");
    let real2 = tmp.path().join("real2.rs");
    std::fs::write(&real1, "fn a() {}\n").unwrap();
    std::fs::write(&real2, "fn b() {}\n").unwrap();

    let files = vec![
        real1.clone(),
        real2.clone(),
        PathBuf::from("/nonexistent1.rs"),
        PathBuf::from("/nonexistent2.rs"),
    ];
    QueueProcessor::add_to_queue(&queue_path, &files).unwrap();

    let indexer = test_indexer();
    let processor = QueueProcessor::new(&indexer);
    let result = processor.process_queue(&queue_path).await.unwrap();

    assert_eq!(result.processed, 2);
    assert_eq!(result.failed, 2);
    assert_eq!(
        result.remaining, 2,
        "remaining should equal number of failed entries left in queue"
    );

    // Cross-check with actual file
    let entries = read_queue_json(&queue_path);
    assert_eq!(
        entries.len(),
        result.remaining,
        "remaining in result should match actual file entries"
    );
}
