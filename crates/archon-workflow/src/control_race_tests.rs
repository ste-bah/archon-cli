//! A stop has to reach work that is already in flight.

use super::*;
use crate::spec::WORKFLOW_SCHEMA;

fn probe_spec() -> crate::WorkflowSpec {
    crate::WorkflowSpec {
        schema: WORKFLOW_SCHEMA.to_string(),
        name: "control-race-test".to_string(),
        task: "test".to_string(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: std::collections::BTreeMap::new(),
        learning_hooks: Vec::new(),
    }
}

/// The ordinary case must be untouched: a call that finishes returns its own
/// answer, and the watcher costs it nothing.
#[tokio::test]
async fn work_that_finishes_returns_its_own_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(probe_spec()).expect("run");

    let answer = until_run_stops(&store, &run.id, "call-1", async { Ok(7u32) })
        .await
        .expect("an uncancelled call returns normally");

    assert_eq!(answer, 7);
}

/// The regression: a cancel used to be invisible until the call returned, so a
/// call that never returns held the run open indefinitely. This one would hang
/// forever without the watcher.
#[tokio::test(start_paused = true)]
async fn a_cancel_abandons_a_call_that_is_still_running() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(probe_spec()).expect("run");
    crate::LifecycleController::new(store.clone())
        .apply(&run.id, crate::LifecycleAction::Cancel)
        .expect("cancel the run");

    let never = async {
        std::future::pending::<()>().await;
        Ok(0u32)
    };
    let error = until_run_stops(&store, &run.id, "call-1", never)
        .await
        .expect_err("a cancelled run must abandon a call that is still in flight");

    assert!(
        matches!(error, WorkflowError::ControlCancelled(_)),
        "a stop must surface as the typed control error, got: {error:?}"
    );
}

/// A running run must not be stopped by the watcher — otherwise every long call
/// would be killed after one poll interval.
#[tokio::test(start_paused = true)]
async fn a_running_run_lets_a_slow_call_continue() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(probe_spec()).expect("run");

    let slow = async {
        // Several poll intervals: each one must decline to stop it.
        tokio::time::sleep(CONTROL_POLL_INTERVAL * 4).await;
        Ok(11u32)
    };
    let answer = until_run_stops(&store, &run.id, "call-1", slow)
        .await
        .expect("a running run must not abandon its own work");

    assert_eq!(answer, 11);
}

/// An unreadable state file is a transient, not the operator's intent. Reading
/// it as a stop would kill live work over a torn read.
#[test]
fn an_unreadable_state_file_is_not_a_stop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));

    assert!(!run_has_stopped(&store, "no-such-run"));
}
