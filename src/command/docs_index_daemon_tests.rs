//! Issue 342: `stop` removes a pid file whose pid no longer names the
//! daemon, and never signals a process this user does not own.

use super::{PidOwner, pid_owner, stop_pid_file};
use archon_test_support::live_process::LiveChild;

fn pid_file(dir: &tempfile::TempDir, pid: u32) -> std::path::PathBuf {
    let path = dir.path().join("docs-index-daemon.pid");
    std::fs::write(&path, pid.to_string()).unwrap();
    path
}

/// Pid 1 serves here only as a live process an unprivileged user cannot
/// signal, as when the OS gives the daemon's pid to another user's process.
#[cfg(unix)]
#[test]
fn stop_removes_a_pid_file_naming_another_users_process_and_sends_no_signal() {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: root may signal pid 1, so it is not another user's process");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = pid_file(&dir, 1);
    assert_eq!(pid_owner(1), PidOwner::Foreign);

    let line = stop_pid_file(&path).unwrap();

    assert!(line.contains("no signal was sent"), "{line}");
    assert!(!path.exists(), "the stale pid file is removed");
    // `start` and `status` read a foreign pid as not the daemon.
    assert_ne!(pid_owner(1), PidOwner::Ours);
}

#[test]
fn stop_removes_a_pid_file_whose_process_was_reaped() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = LiveChild::spawn();
    let path = pid_file(&dir, child.pid());
    child.end();
    assert_eq!(pid_owner(child.pid()), PidOwner::Gone);

    let line = stop_pid_file(&path).unwrap();

    assert!(line.starts_with("Removed stale"), "{line}");
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn stop_terminates_a_running_daemon_this_user_owns() {
    let dir = tempfile::tempdir().unwrap();
    let child = LiveChild::spawn();
    let path = pid_file(&dir, child.pid());
    assert_eq!(pid_owner(child.pid()), PidOwner::Ours);

    let line = stop_pid_file(&path).unwrap();

    assert_eq!(
        line,
        format!("Stopped docs index daemon pid {}.", child.pid())
    );
    assert!(!path.exists());
}

#[test]
fn stop_without_a_pid_file_reports_not_running() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("docs-index-daemon.pid");
    assert_eq!(
        stop_pid_file(&path).unwrap(),
        "Docs index daemon is not running."
    );
}
