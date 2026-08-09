use archon_cognitive::{
    CognitiveDaemon, CognitiveDaemonConfig, DaemonJob, DaemonJobReport, DaemonPaths, DaemonState,
    PersistentCognitiveStore, SituationKind,
};
use archon_policy::CognitivePolicy;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn policy(allow_daemon: bool) -> CognitivePolicy {
    CognitivePolicy {
        enabled: true,
        allow_autonomous_tick: true,
        allow_background_daemon: allow_daemon,
        ..Default::default()
    }
}

fn config() -> CognitiveDaemonConfig {
    CognitiveDaemonConfig {
        enabled: true,
        interval_ms: 5_000,
        stale_heartbeat_ms: 30_000,
        run_on_start: true,
        max_ticks_per_run: 1,
        // Off for the existing tests, so their behaviour is unchanged.
        idle_exit_ms: 0,
    }
}

fn seed_reflection(db: &cozo::DbInstance) {
    archon_cognitive::ensure_cognitive_schema(db).unwrap();
    let script = format!(
        "?[reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at] <- [[\"r1\", \"s1\", 1, \"d1\", \"{}\", \"try\", \"ok\", \"\", \"success\", \"prefer safe probes\", true, \"\", \"2026-05-25T00:00:00Z\"]]\n\
         :put cognitive_reflections {{ reflection_id => session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at }}",
        SituationKind::CiDebug.as_str()
    );
    db.run_script(&script, Default::default(), cozo::ScriptMutability::Mutable)
        .unwrap();
}

#[test]
fn daemon_status_is_not_running_without_state() {
    let dir = tempfile::tempdir().unwrap();
    let status = CognitiveDaemon::status(dir.path(), 30_000).unwrap();
    assert!(!status.running);
    assert!(!status.stale);
    assert!(status.state.is_none());
}

#[test]
fn daemon_status_treats_dead_pid_lock_as_stale() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DaemonPaths::new(dir.path());
    let mut state = DaemonState::new();
    state.pid = u32::MAX;
    paths.write_state(&state).unwrap();
    std::fs::write(&paths.lock_path, "dead\n").unwrap();

    let status = CognitiveDaemon::status(dir.path(), 30_000).unwrap();

    assert!(!status.running);
    assert!(status.stale);
}

#[test]
fn daemon_run_once_clears_fresh_dead_pid_lock() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DaemonPaths::new(dir.path());
    let mut state = DaemonState::new();
    state.pid = u32::MAX;
    paths.write_state(&state).unwrap();
    std::fs::write(&paths.lock_path, "pid=4294967295\n").unwrap();

    let store = PersistentCognitiveStore::open(dir.path()).unwrap();
    let mut daemon = CognitiveDaemon::new(dir.path(), config(), store.db(), policy(true));
    let state = daemon.run_once().unwrap();

    assert_eq!(state.ticks_run, 1);
    assert!(!paths.lock_path.exists());
}

#[test]
fn daemon_run_once_requires_daemon_policy() {
    let dir = tempfile::tempdir().unwrap();
    let store = PersistentCognitiveStore::open(dir.path()).unwrap();
    let mut daemon = CognitiveDaemon::new(dir.path(), config(), store.db(), policy(false));
    let error = daemon.run_once().unwrap_err().to_string();
    assert!(error.contains("background daemon"));
}

#[test]
fn daemon_run_once_records_tick_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = PersistentCognitiveStore::open(dir.path()).unwrap();
    seed_reflection(store.db());
    let mut daemon = CognitiveDaemon::new(dir.path(), config(), store.db(), policy(true));
    let state = daemon.run_once().unwrap();
    assert_eq!(state.ticks_run, 1);
    assert_eq!(state.status, "stopped");
    assert!(state.current_job.is_none());
    assert!(state.tick_started_at.is_none());

    let status = CognitiveDaemon::status(dir.path(), 30_000).unwrap();
    assert!(!status.running);
    assert_eq!(status.state.unwrap().ticks_run, 1);
}

#[test]
fn daemon_stop_writes_stop_marker() {
    let dir = tempfile::tempdir().unwrap();
    CognitiveDaemon::request_stop(dir.path()).unwrap();
    let status = CognitiveDaemon::status(dir.path(), 30_000).unwrap();
    assert!(status.stop_requested);
}

#[test]
fn daemon_run_forever_consumes_stop_marker_on_shutdown() {
    struct StopJob {
        root: PathBuf,
    }

    impl DaemonJob for StopJob {
        fn name(&self) -> &'static str {
            "stop_job"
        }

        fn run(&mut self) -> Result<DaemonJobReport, archon_cognitive::CognitiveError> {
            CognitiveDaemon::request_stop(&self.root)?;
            Ok(DaemonJobReport {
                name: self.name().into(),
                ok: true,
                summary: "requested stop".into(),
            })
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let store = PersistentCognitiveStore::open(dir.path()).unwrap();
    let mut daemon = CognitiveDaemon::new(dir.path(), config(), store.db(), policy(true));
    daemon.add_job(StopJob {
        root: dir.path().to_path_buf(),
    });
    let state = daemon.run_forever().unwrap();
    assert_eq!(state.status, "stopped");

    let status = CognitiveDaemon::status(dir.path(), 30_000).unwrap();
    assert!(!status.stop_requested);
}

#[test]
fn daemon_state_reads_remain_valid_during_heartbeat_writes() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DaemonPaths::new(dir.path());
    let done = Arc::new(AtomicBool::new(false));
    let writer_done = Arc::clone(&done);
    let writer_paths = paths.clone();
    let writer = std::thread::spawn(move || {
        let mut state = DaemonState::new();
        for _ in 0..2_000 {
            state.heartbeat();
            writer_paths.write_state(&state).unwrap();
        }
        writer_done.store(true, Ordering::Release);
    });

    while !done.load(Ordering::Acquire) {
        let _ = paths.read_state().expect("state is always valid JSON");
    }
    writer.join().unwrap();
}

#[test]
fn daemon_heartbeats_while_job_blocks() {
    struct SlowJob;

    impl DaemonJob for SlowJob {
        fn name(&self) -> &'static str {
            "slow_job"
        }

        fn run(&mut self) -> Result<DaemonJobReport, archon_cognitive::CognitiveError> {
            std::thread::sleep(Duration::from_millis(1_800));
            Ok(DaemonJobReport {
                name: self.name().into(),
                ok: true,
                summary: "slept".into(),
            })
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let thread_root = root.clone();
    let handle = std::thread::spawn(move || {
        let store = PersistentCognitiveStore::open(&thread_root).unwrap();
        let mut daemon_config = config();
        daemon_config.stale_heartbeat_ms = 1_000;
        let mut daemon =
            CognitiveDaemon::new(&thread_root, daemon_config, store.db(), policy(true));
        daemon.add_job(SlowJob);
        daemon.run_once().unwrap();
    });

    let started = wait_for_job(&root, "slow_job");
    std::thread::sleep(Duration::from_millis(1_200));
    let updated = CognitiveDaemon::status(&root, 1_000)
        .unwrap()
        .state
        .unwrap()
        .last_heartbeat_at;

    handle.join().unwrap();

    assert!(updated > started);
}

/// Wait for the daemon thread to pick up `name`.
///
/// The budget is generous on purpose: this is a positive wait that returns as
/// soon as the job appears, so a large ceiling costs nothing on a fast machine.
/// At the previous 2s it failed on a loaded Windows runner at 2.19s -- the
/// daemon had simply not been scheduled yet.
fn wait_for_job(root: &std::path::Path, name: &str) -> chrono::DateTime<chrono::Utc> {
    for _ in 0..300 {
        if let Some(state) = CognitiveDaemon::status(root, 1_000).unwrap().state
            && state.current_job.as_deref() == Some(name)
        {
            return state.last_heartbeat_at;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("job {name} did not start");
}

/// The daemon `setsid()`s away from its launcher on purpose, so it outlives the
/// session — that is what a daemon is for. What it lacked was any notion of
/// being ABANDONED: `run_forever` had no exit but an explicit stop file, so a
/// run that ended without one left it behind. One was found alive after 10h43m,
/// reparented to init, still holding a deleted copy of a binary replaced three
/// times since; four accumulated in a day.
///
/// The launcher already wrote an activity marker. Nothing read it. Now it does.
#[test]
fn daemon_exits_when_archon_activity_has_gone_stale() {
    let dir = tempfile::tempdir().unwrap();
    let store = PersistentCognitiveStore::open(dir.path()).unwrap();
    let paths = DaemonPaths::new(dir.path());
    paths.ensure_root().unwrap();

    // An activity marker from well before the idle window.
    let stale = chrono::Utc::now() - chrono::Duration::hours(3);
    std::fs::write(
        &paths.activity_path,
        serde_json::to_vec(&serde_json::json!({
            "pid": 1234,
            "reason": "session turn",
            "updated_at": stale,
        }))
        .unwrap(),
    )
    .unwrap();

    let mut cfg = config();
    // Bounded, deliberately. `stopped_idle` is set ONLY by the idle rule, so
    // the status still proves which path ended the run — while the tick cap
    // guarantees the test terminates either way.
    //
    // The first version left this unbounded so "only the idle rule can stop
    // it". That makes a regression HANG rather than fail: sabotaging the check
    // while writing this left a test binary spinning at 39% CPU for two and a
    // half hours before anyone noticed. A test whose failure mode is "runs
    // forever" is worse than no test.
    cfg.max_ticks_per_run = 50;
    cfg.interval_ms = 10;
    cfg.idle_exit_ms = 60_000;

    let mut daemon = CognitiveDaemon::new(dir.path(), cfg, store.db(), policy(true));
    let state = daemon.run_forever().unwrap();
    assert_eq!(
        state.status, "stopped_idle",
        "an abandoned daemon must stop itself; `stopped` here means it ran to \
         the tick cap instead and the idle rule never fired"
    );
}

/// A live session must keep its daemon. Without this the fix would trade an
/// orphan problem for a daemon that dies under a user who is still working.
#[test]
fn daemon_keeps_running_while_archon_is_active() {
    let dir = tempfile::tempdir().unwrap();
    let store = PersistentCognitiveStore::open(dir.path()).unwrap();
    let paths = DaemonPaths::new(dir.path());
    paths.ensure_root().unwrap();
    std::fs::write(
        &paths.activity_path,
        serde_json::to_vec(&serde_json::json!({
            "pid": 1234,
            "reason": "session turn",
            "updated_at": chrono::Utc::now(),
        }))
        .unwrap(),
    )
    .unwrap();

    let mut cfg = config();
    cfg.interval_ms = 10;
    cfg.idle_exit_ms = 60_000;
    cfg.max_ticks_per_run = 1; // bounded so the test terminates

    let mut daemon = CognitiveDaemon::new(dir.path(), cfg, store.db(), policy(true));
    let state = daemon.run_forever().unwrap();
    assert_ne!(
        state.status, "stopped_idle",
        "a fresh activity marker must not read as abandonment"
    );
}

/// Fails CLOSED. A missing or unreadable marker means we cannot tell, and a
/// transient filesystem problem must not look like abandonment and kill a
/// daemon doing real work.
#[test]
fn an_unreadable_activity_marker_does_not_stop_the_daemon() {
    for marker in [None, Some("not json at all"), Some("{\"updated_at\": 42}")] {
        let dir = tempfile::tempdir().unwrap();
        let store = PersistentCognitiveStore::open(dir.path()).unwrap();
        let paths = DaemonPaths::new(dir.path());
        paths.ensure_root().unwrap();
        if let Some(body) = marker {
            std::fs::write(&paths.activity_path, body).unwrap();
        }

        let mut cfg = config();
        cfg.interval_ms = 10;
        cfg.idle_exit_ms = 1; // would fire instantly if the marker parsed
        cfg.max_ticks_per_run = 1;

        let mut daemon = CognitiveDaemon::new(dir.path(), cfg, store.db(), policy(true));
        let state = daemon.run_forever().unwrap();
        assert_ne!(
            state.status, "stopped_idle",
            "unreadable marker {marker:?} must not be treated as abandonment"
        );
    }
}
