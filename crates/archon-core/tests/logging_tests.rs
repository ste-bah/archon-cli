use std::fs;
use std::path::PathBuf;

use archon_core::logging::{init_logging, rotate_logs};

fn temp_log_dir() -> PathBuf {
    let dir = std::env::temp_dir()
        .join("archon-test-logs")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("should create temp log dir");
    dir
}

fn cleanup(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

/// Single init_logging test -- tracing can only be initialized once per process.
/// This test covers both file creation AND directory creation.
#[test]
fn init_logging_creates_file_and_directories() {
    // Use a nested path that doesn't exist yet
    let dir = std::env::temp_dir()
        .join("archon-test-logs")
        .join(uuid::Uuid::new_v4().to_string())
        .join("nested")
        .join("deep");

    assert!(!dir.exists(), "dir should not exist before init");

    let session_id = "test-session-001";
    let guard = init_logging(session_id, "info", &dir)
        .expect("init_logging should succeed");

    // Directory was created
    assert!(dir.exists(), "log directory should be created by init_logging");

    // Log file was created
    let log_file = dir.join(format!("{session_id}.log"));
    assert!(log_file.exists(), "log file should be created at {log_file:?}");

    drop(guard);
    let _ = fs::remove_dir_all(
        dir.parent().and_then(|p| p.parent()).expect("grandparent"),
    );
}

#[test]
fn rotate_logs_keeps_max_files() {
    let dir = temp_log_dir();

    // Create 10 log files with staggered mtimes
    for i in 0..10 {
        let path = dir.join(format!("session-{i:03}.log"));
        fs::write(&path, format!("log content {i}"))
            .expect("should write test log file");
        // Small sleep to ensure different mtimes
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Rotate keeping only 5
    rotate_logs(&dir, 5).expect("rotate_logs should succeed");

    let remaining: Vec<_> = fs::read_dir(&dir)
        .expect("should read dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "log"))
        .collect();

    assert_eq!(
        remaining.len(),
        5,
        "should keep exactly 5 files, got {}",
        remaining.len()
    );

    // The 5 newest files (session-005 through session-009) should survive
    for i in 5..10 {
        let path = dir.join(format!("session-{i:03}.log"));
        assert!(path.exists(), "newest file {path:?} should survive rotation");
    }

    cleanup(&dir);
}

#[test]
fn rotate_logs_noop_when_under_limit() {
    let dir = temp_log_dir();

    for i in 0..3 {
        fs::write(dir.join(format!("s{i}.log")), "data").expect("should write");
    }

    rotate_logs(&dir, 5).expect("rotate should succeed");

    let count = fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|e| e.ok())
        .count();

    assert_eq!(count, 3, "all files should remain when under limit");

    cleanup(&dir);
}

#[test]
fn rotate_logs_handles_empty_directory() {
    let dir = temp_log_dir();
    rotate_logs(&dir, 50).expect("rotate on empty dir should succeed");
    cleanup(&dir);
}

#[test]
fn rotate_logs_handles_nonexistent_directory() {
    let dir = std::env::temp_dir()
        .join("archon-test-logs")
        .join("does-not-exist-at-all");

    rotate_logs(&dir, 50).expect("rotate on missing dir should succeed");
}
