//! Issue 342: the daemon lock is reclaimed only when its holder is gone, on
//! every platform. The pid probe answered "dead" for every pid off Unix, so
//! a second daemon removed a running daemon's lock there and `status`
//! reported a running daemon as stale.

use archon_cognitive::{
    CognitiveDaemon, CognitiveDaemonConfig, DaemonPaths, DaemonState, PersistentCognitiveStore,
};
use archon_policy::CognitivePolicy;
use archon_test_support::live_process::LiveChild;

const STALE_MS: u64 = 30_000;

fn policy() -> CognitivePolicy {
    CognitivePolicy {
        enabled: true,
        allow_autonomous_tick: true,
        allow_background_daemon: true,
        ..Default::default()
    }
}

fn config() -> CognitiveDaemonConfig {
    CognitiveDaemonConfig {
        enabled: true,
        interval_ms: 5_000,
        stale_heartbeat_ms: STALE_MS,
        run_on_start: true,
        max_ticks_per_run: 1,
        idle_exit_ms: 0,
    }
}

/// Writes a fresh-heartbeat state and a lock that both name `pid`, as a
/// running daemon leaves them, and returns the lock's contents.
fn hold_lock(paths: &DaemonPaths, pid: u32) -> String {
    let mut state = DaemonState::new();
    state.pid = pid;
    paths.write_state(&state).unwrap();
    let lock = format!("pid={pid}\n");
    std::fs::write(&paths.lock_path, &lock).unwrap();
    lock
}

#[test]
fn status_reports_a_running_holder_as_running_not_stale() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DaemonPaths::new(dir.path());
    let holder = LiveChild::spawn();
    hold_lock(&paths, holder.pid());

    let status = CognitiveDaemon::status(dir.path(), STALE_MS).unwrap();

    assert!(status.running, "pid {} still runs", holder.pid());
    assert!(!status.stale);
}

#[test]
fn a_second_daemon_does_not_take_over_a_running_holders_lock() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DaemonPaths::new(dir.path());
    let holder = LiveChild::spawn();
    let lock = hold_lock(&paths, holder.pid());

    let store = PersistentCognitiveStore::open(dir.path()).unwrap();
    let mut daemon = CognitiveDaemon::new(dir.path(), config(), store.db(), policy());
    let error = daemon.run_once().unwrap_err().to_string();

    assert!(error.contains("already running or locked"), "{error}");
    assert_eq!(
        std::fs::read_to_string(&paths.lock_path).unwrap(),
        lock,
        "the running holder's lock must be left as it was"
    );
}

#[test]
fn a_reaped_holders_lock_is_reclaimed_and_reported_stale() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DaemonPaths::new(dir.path());
    let mut holder = LiveChild::spawn();
    hold_lock(&paths, holder.pid());
    holder.end();

    let status = CognitiveDaemon::status(dir.path(), STALE_MS).unwrap();
    assert!(
        !status.running,
        "pid {} was killed and reaped",
        holder.pid()
    );
    assert!(status.stale);

    let store = PersistentCognitiveStore::open(dir.path()).unwrap();
    let mut daemon = CognitiveDaemon::new(dir.path(), config(), store.db(), policy());
    let state = daemon.run_once().unwrap();
    assert_eq!(state.ticks_run, 1);
    assert!(
        !paths.lock_path.exists(),
        "the run releases the lock it took"
    );
}
