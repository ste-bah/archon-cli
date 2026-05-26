use archon_cognitive::{
    CognitiveDaemon, CognitiveDaemonConfig, PersistentCognitiveStore, SituationKind,
};
use archon_policy::CognitivePolicy;

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
