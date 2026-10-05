//! Issue 291: a session bound to the executor that started under generation
//! g writes nothing once a resume hands the run to a newer executor.
use super::*;
use crate::{LifecycleAction, LifecycleController, RunStatus, WorkflowStore};

fn outcome() -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: "one".into(),
        role: "worker".into(),
        status: WorkflowV2Status::Accepted,
        result: Some(WorkflowV2Result::accepted("done")),
        error: None,
        failure_kind: None,
        item_input_hash: Some("input".into()),
        completion_evidence: vec![],
    }
}

/// A running run, and a store bound to the executor that started it.
fn bound_session() -> (
    tempfile::TempDir,
    WorkflowStore,
    String,
    WorkflowV2ResultStore,
) {
    let temp = tempfile::tempdir().unwrap();
    let runs = WorkflowStore::new(temp.path().join("workflows"));
    let spec = crate::WorkflowSpec {
        schema: crate::spec::WORKFLOW_SCHEMA.into(),
        name: "owner".into(),
        task: "test".into(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    };
    let mut run = runs.create_run(spec).unwrap();
    run.status = RunStatus::Running;
    runs.save_state(&run).unwrap();
    let v2 = WorkflowV2ResultStore::new(runs.run_dir(&run.id).join("v2"));
    v2.bind_session_executor(run.generation);
    (temp, runs, run.id, v2)
}

fn take_over(runs: &WorkflowStore, run_id: &str) {
    let lifecycle = LifecycleController::new(runs.clone());
    lifecycle.apply(run_id, LifecycleAction::Pause).unwrap();
    lifecycle.apply(run_id, LifecycleAction::Resume).unwrap();
}

#[test]
fn the_owning_session_writes_and_a_pause_keeps_it_the_owner() {
    let (_temp, runs, run_id, v2) = bound_session();
    v2.save_branch_outcome("fanout", &outcome()).unwrap();
    LifecycleController::new(runs.clone())
        .apply(&run_id, LifecycleAction::Pause)
        .unwrap();
    assert!(
        v2.require_session_owner().is_ok(),
        "a pause keeps the executor"
    );
}

#[test]
fn a_stale_session_writes_no_branch_outcome_after_a_resume() {
    let (_temp, runs, run_id, v2) = bound_session();
    take_over(&runs, &run_id);
    let refused = v2.save_branch_outcome("fanout", &outcome());
    assert!(
        matches!(&refused, Err(WorkflowError::ControlCancelled(message))
            if message.contains("no longer owns") && message.contains("stale session")),
        "{refused:?}"
    );
    assert!(v2.load_branch_outcome("fanout", "one").unwrap().is_none());
}

#[test]
fn an_unbound_store_is_not_fenced() {
    let (_temp, runs, run_id, _bound) = bound_session();
    take_over(&runs, &run_id);
    let unbound = WorkflowV2ResultStore::new(runs.run_dir(&run_id).join("v2"));
    assert!(unbound.require_session_owner().is_ok());
}

#[test]
fn the_first_binding_stands_for_every_clone() {
    let (_temp, _runs, _run_id, v2) = bound_session();
    let first = v2.session_executor();
    v2.clone().bind_session_executor(first.unwrap() + 7);
    assert_eq!(v2.session_executor(), first);
}
