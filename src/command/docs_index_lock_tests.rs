//! Issue 342: the docs index lock is stale only when its writer is gone, on
//! every platform. It used to answer "running" for every pid off Unix, so a
//! dead writer's lock was never reclaimed there.

use super::stale_lock;
use archon_test_support::live_process::LiveChild;

fn lock_naming(dir: &tempfile::TempDir, body: &str) -> std::path::PathBuf {
    let path = dir.path().join("docs-index.lock");
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn a_lock_held_by_a_running_writer_is_not_stale() {
    let dir = tempfile::tempdir().unwrap();
    let writer = LiveChild::spawn();
    let path = lock_naming(&dir, &format!("pid={}\n", writer.pid()));
    assert!(!stale_lock(&path), "writer {} still runs", writer.pid());
}

#[test]
fn a_lock_whose_writer_was_reaped_is_stale() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = LiveChild::spawn();
    let path = lock_naming(&dir, &format!("pid={}\n", writer.pid()));
    writer.end();
    assert!(
        stale_lock(&path),
        "writer {} was killed and reaped",
        writer.pid()
    );
}

#[test]
fn a_lock_held_by_this_process_is_not_stale() {
    let dir = tempfile::tempdir().unwrap();
    let path = lock_naming(&dir, &format!("pid={}\n", std::process::id()));
    assert!(!stale_lock(&path));
}

#[test]
fn a_lock_without_a_readable_pid_is_not_proven_stale() {
    let dir = tempfile::tempdir().unwrap();
    let path = lock_naming(&dir, "garbage\n");
    assert!(!stale_lock(&path));
}
