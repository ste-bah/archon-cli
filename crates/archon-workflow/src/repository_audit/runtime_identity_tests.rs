//! Issue-83: a run that took a lifecycle action must still be able to run its
//! write stages, and a state file that is actually broken must still say so.
use super::budget::{AuditPolicy, Limit};
use super::runtime::{AuditRuntime, STATE_PATH};
use crate::{
    LifecycleAction, LifecycleController, WorkflowError, WorkflowRun, WorkflowSpec, WorkflowStore,
    spec,
};

fn policy() -> AuditPolicy {
    AuditPolicy {
        attempt_timeout_secs: Limit::Unlimited,
        total_time_secs: Limit::Unlimited,
        unexpected_change_refreshes: Limit::Unlimited,
    }
}

fn run_store() -> (tempfile::TempDir, WorkflowStore, WorkflowRun) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(WorkflowSpec {
            schema: spec::WORKFLOW_SCHEMA.into(),
            name: "identity".into(),
            task: "audit".into(),
            target_repository_root: None,
            max_agents: 1,
            max_parallelism: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    (temp, store, run)
}

/// Any lifecycle action bumps the run's generation.
fn bump(store: &WorkflowStore, run_id: &str) {
    LifecycleController::new(store.clone())
        .apply(run_id, LifecycleAction::Pause)
        .unwrap();
}

fn generation(store: &WorkflowStore, run_id: &str) -> u64 {
    store.load_state(run_id).unwrap().generation
}

/// Rewrite the state file through plain JSON, to forge shapes the typed
/// writer could never produce.
fn corrupt(store: &WorkflowStore, run_id: &str, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = store.run_dir(run_id).join(STATE_PATH);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    edit(&mut value);
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
}

/// The live shape: a long-lived handle built at one generation, a lifecycle
/// action after it, and a fresh executor re-establishing the audit state at
/// the new generation. The stale handle used to call that corruption and take
/// every write stage down with it.
#[test]
fn a_generation_bump_under_a_live_handle_is_not_corruption() {
    let (_temp, store, run) = run_store();
    let audit = AuditRuntime::initialize(store.clone(), run.id.clone(), policy()).unwrap();
    let built_at = audit.generation;
    bump(&store, &run.id);
    AuditRuntime::initialize(store.clone(), run.id.clone(), policy()).unwrap();
    assert!(built_at < generation(&store, &run.id), "the run moved on");

    let state = audit.state().expect("a stale handle still reads the state");
    assert_eq!(state.generation, generation(&store, &run.id));
    // And the write path that reads it proceeds rather than failing.
    audit
        .update(|state| {
            state.attempts += 1;
            Ok(())
        })
        .expect("a stale handle still writes");
    assert_eq!(audit.state().unwrap().attempts, 1);
}

/// The audit state left behind at an older generation than the run: refused,
/// but as the control condition that ends the executor and invites a
/// re-dispatch — never as corruption charged to the asking stage.
#[test]
fn an_audit_state_the_run_moved_past_is_a_control_pause() {
    let (_temp, store, run) = run_store();
    let audit = AuditRuntime::initialize(store.clone(), run.id.clone(), policy()).unwrap();
    // Nothing re-establishes the state, so the file stays behind the run.
    bump(&store, &run.id);
    assert!(
        matches!(audit.state(), Err(WorkflowError::ControlPaused(_))),
        "{:?}",
        audit.state().err()
    );
    assert!(matches!(
        audit.update(|_| Ok(())),
        Err(WorkflowError::ControlPaused(_))
    ));
}

/// Real corruption is still corruption, in every shape the file can take it.
#[test]
fn an_unreadable_or_wrong_schema_audit_state_is_still_corrupt() {
    let (_temp, store, run) = run_store();
    let audit = AuditRuntime::initialize(store.clone(), run.id.clone(), policy()).unwrap();
    audit.state().expect("sound to begin with");

    corrupt(&store, &run.id, |value| value["schema_version"] = 2.into());
    assert!(
        matches!(audit.state(), Err(WorkflowError::StateCorrupt(_))),
        "wrong schema"
    );

    corrupt(&store, &run.id, |value| {
        value["schema_version"] = 1.into();
        value["unexpected_field"] = "from a newer build".into();
    });
    assert!(
        matches!(audit.state(), Err(WorkflowError::StateCorrupt(_))),
        "unknown field"
    );

    // Truncated mid-write: a bare serialiser error before, corruption now.
    let path = store.run_dir(&run.id).join(STATE_PATH);
    std::fs::write(&path, b"{\"schema_version\":1,\"generation\":").unwrap();
    let error = audit.state().expect_err("truncated state must be refused");
    assert!(
        matches!(error, WorkflowError::StateCorrupt(_)),
        "truncated: {error:?}"
    );
    assert!(
        error.to_string().contains(&run.id),
        "the message names the run: {error}"
    );
}
