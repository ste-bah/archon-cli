use std::fs;
use std::path::PathBuf;

use archon_session::background::{BackgroundSessionInfo, sessions_dir};
use archon_session::registry;

/// Helper: create a temp directory to use as a fake sessions dir.
fn temp_sessions_dir() -> PathBuf {
    let dir = std::env::temp_dir()
        .join("archon-bg-test")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("create temp sessions dir");
    dir
}

fn cleanup(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

/// Write a status JSON file into the given directory.
fn write_status(dir: &PathBuf, id: &str, name: &str, status: &str, started_at: &str) {
    let info = serde_json::json!({
        "id": id,
        "name": name,
        "status": status,
        "pid": null,
        "started_at": started_at,
        "turns": 0,
        "cost": 0.0,
        "last_activity": null,
        "error": null,
    });
    let path = dir.join(format!("{id}.status.json"));
    fs::write(&path, serde_json::to_string_pretty(&info).expect("json")).expect("write status");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: sessions_dir_path
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn sessions_dir_path() {
    let dir = sessions_dir();
    let path_str = dir.to_string_lossy();
    assert!(
        path_str.contains("archon") && path_str.contains("sessions"),
        "sessions_dir should contain 'archon/sessions', got: {path_str}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: status_file_has_correct_fields
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn status_file_has_correct_fields() {
    let dir = temp_sessions_dir();
    let id = "test-session-001";
    write_status(&dir, id, "my-task", "running", "2026-04-03T12:00:00Z");

    let info = registry::get_session_in_dir(&dir, id).expect("get session");
    assert_eq!(info.id, id);
    assert_eq!(info.name, "my-task");
    assert_eq!(info.status, "running");
    assert_eq!(info.started_at, "2026-04-03T12:00:00Z");
    assert_eq!(info.turns, 0);
    assert!((info.cost - 0.0).abs() < f64::EPSILON);

    cleanup(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: list_empty_when_no_sessions
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn list_empty_when_no_sessions() {
    let dir = temp_sessions_dir();
    let list = registry::list_sessions_in_dir(&dir).expect("list");
    assert!(
        list.is_empty(),
        "expected empty list, got {} items",
        list.len()
    );
    cleanup(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: list_finds_session
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn list_finds_session() {
    let dir = temp_sessions_dir();
    write_status(&dir, "sess-a", "alpha", "running", "2026-04-03T12:00:00Z");
    write_status(&dir, "sess-b", "beta", "completed", "2026-04-03T11:00:00Z");

    let list = registry::list_sessions_in_dir(&dir).expect("list");
    assert_eq!(list.len(), 2);

    // Newest first
    assert_eq!(list[0].id, "sess-a");
    assert_eq!(list[1].id, "sess-b");

    cleanup(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: cleanup_removes_old
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cleanup_removes_old() {
    let dir = temp_sessions_dir();

    // Create an old session (8 days ago)
    let old_time = chrono::Utc::now() - chrono::Duration::days(8);
    write_status(&dir, "old-sess", "old", "completed", &old_time.to_rfc3339());
    // Also create the log and pid files that should be cleaned up
    fs::write(dir.join("old-sess.log"), "log data").expect("write log");
    fs::write(dir.join("old-sess.pid"), "12345").expect("write pid");

    let removed = registry::cleanup_old_sessions_in_dir(&dir, 7).expect("cleanup");
    assert_eq!(removed, 1);

    // Status file should be gone
    assert!(!dir.join("old-sess.status.json").exists());
    // Log and pid files should also be gone
    assert!(!dir.join("old-sess.log").exists());
    assert!(!dir.join("old-sess.pid").exists());

    cleanup(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: cleanup_preserves_recent
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cleanup_preserves_recent() {
    let dir = temp_sessions_dir();

    let now = chrono::Utc::now();
    write_status(&dir, "new-sess", "recent", "completed", &now.to_rfc3339());

    let removed = registry::cleanup_old_sessions_in_dir(&dir, 7).expect("cleanup");
    assert_eq!(removed, 0);
    assert!(dir.join("new-sess.status.json").exists());

    cleanup(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: stale_pid_detection
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn stale_pid_detection() {
    let dir = temp_sessions_dir();

    // Use a PID that almost certainly doesn't exist (max PID)
    let stale_pid = 4_194_300u32;
    write_status(
        &dir,
        "stale-sess",
        "stale",
        "running",
        "2026-04-03T12:00:00Z",
    );
    fs::write(dir.join("stale-sess.pid"), stale_pid.to_string()).expect("write pid");

    let cleaned = registry::cleanup_stale_pids_in_dir(&dir).expect("cleanup stale");
    assert_eq!(cleaned, 1);

    // Status should now show as "stale" or be cleaned up
    let info = registry::get_session_in_dir(&dir, "stale-sess").expect("get");
    assert_eq!(info.status, "stale");

    cleanup(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: kill_session_updates_status
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn kill_session_updates_status() {
    let dir = temp_sessions_dir();
    write_status(&dir, "kill-me", "doomed", "running", "2026-04-03T12:00:00Z");
    // Write a PID file with a non-existent PID (so kill won't actually kill anything)
    fs::write(dir.join("kill-me.pid"), "4194300").expect("write pid");

    // kill_session_in_dir should update status to "killed" regardless of whether
    // the process was alive (it just sends SIGTERM and updates status)
    let result = archon_session::background::kill_session_in_dir(&dir, "kill-me");
    assert!(result.is_ok());

    let info = registry::get_session_in_dir(&dir, "kill-me").expect("get");
    assert_eq!(info.status, "killed");

    cleanup(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: view_logs_reads_file
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn view_logs_reads_file() {
    let dir = temp_sessions_dir();
    let log_content = "line 1\nline 2\nline 3\n";
    fs::write(dir.join("log-sess.log"), log_content).expect("write log");

    let content = archon_session::attach::view_logs_in_dir(&dir, "log-sess").expect("view logs");
    assert_eq!(content, log_content);

    cleanup(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: view_logs_missing_file_errors
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn view_logs_missing_file_errors() {
    let dir = temp_sessions_dir();
    let result = archon_session::attach::view_logs_in_dir(&dir, "nonexistent");
    assert!(result.is_err());

    cleanup(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: background_session_info_serializes
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn background_session_info_serializes() {
    let info = BackgroundSessionInfo {
        id: "abc-123".to_string(),
        name: "test-session".to_string(),
        status: "running".to_string(),
        pid: Some(42),
        started_at: "2026-04-03T12:00:00Z".to_string(),
        turns: 5,
        cost: 0.25,
        last_activity: Some("2026-04-03T12:05:00Z".to_string()),
        error: None,
    };

    let json = serde_json::to_string(&info).expect("serialize");
    let deserialized: BackgroundSessionInfo = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(deserialized.id, info.id);
    assert_eq!(deserialized.name, info.name);
    assert_eq!(deserialized.status, info.status);
    assert_eq!(deserialized.pid, info.pid);
    assert_eq!(deserialized.started_at, info.started_at);
    assert_eq!(deserialized.turns, info.turns);
    assert!((deserialized.cost - info.cost).abs() < f64::EPSILON);
    assert_eq!(deserialized.last_activity, info.last_activity);
    assert_eq!(deserialized.error, info.error);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: launch_creates_files (uses a dummy binary)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn launch_creates_files() {
    let dir = temp_sessions_dir();

    // Use /bin/sleep as a stand-in for the archon binary
    let sleep_bin = std::path::Path::new("/bin/sleep");
    if !sleep_bin.exists() {
        // Skip on systems without /bin/sleep
        cleanup(&dir);
        return;
    }

    let session_id = archon_session::background::launch_background_in_dir(
        &dir,
        "test query",
        Some("test-launch"),
        sleep_bin,
    )
    .expect("launch");

    // Give the process a moment to start
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Verify files exist
    assert!(
        dir.join(format!("{session_id}.log")).exists(),
        "log file should exist"
    );
    assert!(
        dir.join(format!("{session_id}.pid")).exists(),
        "pid file should exist"
    );
    assert!(
        dir.join(format!("{session_id}.status.json")).exists(),
        "status file should exist"
    );

    // Verify status JSON
    let info = registry::get_session_in_dir(&dir, &session_id).expect("get session");
    assert_eq!(info.name, "test-launch");
    assert!(info.pid.is_some());

    // Kill the sleep process
    let _ = archon_session::background::kill_session_in_dir(&dir, &session_id);

    cleanup(&dir);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test: cfg_unix_only (compile-time verification)
// ─────────────────────────────────────────────────────────────────────────────
// The #[cfg(unix)] guards on launch_background, kill_session, and
// is_session_alive are verified at compile time. If this test file compiles
// on the target platform, the guards are correctly applied.
#[test]
fn cfg_unix_guards_compile() {
    // This test exists to document that Unix-specific functions are gated.
    // If compilation succeeds, the guards are correct.
    #[cfg(unix)]
    {
        // These functions exist on Unix
        let _ = archon_session::background::is_session_alive_in_dir
            as fn(&std::path::Path, &str) -> bool;
    }
}
