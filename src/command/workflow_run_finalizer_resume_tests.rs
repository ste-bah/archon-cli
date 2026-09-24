//! End to end: a run that reaches a terminal decision after an earlier resume
//! point finalizes instead of dying on a changed-identity refusal.

use super::*;

use archon_workflow::{RunEndObserverStateV1, WorkflowRunKind, WorkflowV2Status};

use super::workflow_live_v2_finalizer::{finalize_run_status, finalize_summary};
use super::workflow_run_finalizer_tests::{
    events, read_finalization, seed_call, snapshot, spec, summary,
};

/// The live defect: a paused run records no v2 terminal status, and the
/// stages of the resumed attempt then fail. The decision from none to a
/// decided status must finalize the run, not report corrupt state.
#[tokio::test]
async fn a_resumed_run_may_decide_a_terminal_status_the_pause_left_open() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Failed);

    finalize_run_status(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        RunStatus::Paused,
        "paused mid run",
        None,
    )
    .unwrap();
    assert!(
        read_finalization(&store, &run.id)
            .terminal_v2_status
            .is_none()
    );

    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        None,
        &summary(WorkflowV2Status::Failed),
        &v2_store,
        None,
        None,
    )
    .await
    .expect("a paused run must be able to decide its terminal status");

    let record = read_finalization(&store, &run.id);
    assert_eq!(record.terminal_v2_status, Some(WorkflowV2Status::Failed));
    assert_eq!(record.terminal_status, RunStatus::Failed);
    assert!(record.terminal_event_committed);
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Failed);
    assert!(
        events(&store, &run.id)
            .iter()
            .any(|event| event.detail["event"] == "terminal_status"
                && event.detail["status"] == "failed"),
        "the decided status must reach the event log"
    );
}

/// The same run status arriving twice stays a single committed outcome.
#[tokio::test]
async fn an_unchanged_identity_still_finalizes_without_committing_twice() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Failed);

    for _ in 0..2 {
        finalize_summary(
            &store,
            &run.id,
            WorkflowRunKind::AuthoredTaskWorkflow,
            None,
            &summary(WorkflowV2Status::Failed),
            &v2_store,
            None,
            None,
        )
        .await
        .expect("an unchanged identity must finalize");
    }

    assert_eq!(
        events(&store, &run.id)
            .iter()
            .filter(|event| event.detail["event"] == "terminal_status")
            .count(),
        1
    );
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Failed);
}

/// The capability the guard exists for: a record belonging to another
/// workflow is still refused, whatever its terminal status says.
#[tokio::test]
async fn state_from_another_run_kind_is_still_refused_as_corrupt() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Failed);

    finalize_run_status(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        RunStatus::Paused,
        "paused mid run",
        None,
    )
    .unwrap();

    let error = finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::FixedOrSavedScript,
        None,
        &summary(WorkflowV2Status::Failed),
        &v2_store,
        None,
        None,
    )
    .await
    .expect_err("a different run kind is state from another workflow");
    assert!(error.to_string().contains("identity changed"), "{error}");
    assert!(
        read_finalization(&store, &run.id)
            .terminal_v2_status
            .is_none()
    );
}

/// A completing outcome ends the run, so nothing that runs later may
/// contradict it; every other decided outcome leaves the run resumable and may
/// be superseded by the attempt that follows.
#[tokio::test]
async fn a_completed_outcome_is_final_while_an_open_one_may_be_superseded() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Accepted);

    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        None,
        &summary(WorkflowV2Status::Failed),
        &v2_store,
        None,
        None,
    )
    .await
    .expect("first decision");

    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        None,
        &summary(WorkflowV2Status::Accepted),
        &v2_store,
        None,
        None,
    )
    .await
    .expect("a failed run stays resumable, so a later attempt may accept it");
    let record = read_finalization(&store, &run.id);
    assert_eq!(record.terminal_v2_status, Some(WorkflowV2Status::Accepted));
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Completed
    );
    assert!(record.observer_state.is_none());

    let error = finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        None,
        &summary(WorkflowV2Status::Failed),
        &v2_store,
        None,
        None,
    )
    .await
    .expect_err("a completed run refuses resume, so nothing may contradict it");
    assert!(error.to_string().contains("completed"), "{error}");
    assert_eq!(
        read_finalization(&store, &run.id).terminal_v2_status,
        Some(WorkflowV2Status::Accepted)
    );

    let error = finalize_run_status(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        RunStatus::Paused,
        "paused after completion",
        None,
    )
    .expect_err("a completed run cannot be reopened by a run-status finalization");
    assert!(error.to_string().contains("completed"), "{error}");
}

/// The mirror of the live defect: an outcome recorded without a v2 decision
/// may itself be superseded by a later run-status finalization.
#[test]
fn a_resume_point_may_be_superseded_by_a_later_run_status() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();

    finalize_run_status(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        RunStatus::Paused,
        "paused mid run",
        None,
    )
    .unwrap();
    finalize_run_status(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        RunStatus::Cancelled,
        "cancelled after resume",
        None,
    )
    .expect("a paused run may later be cancelled");

    let record = read_finalization(&store, &run.id);
    assert_eq!(record.terminal_status, RunStatus::Cancelled);
    assert!(record.terminal_v2_status.is_none());
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Cancelled
    );
    assert_eq!(
        events(&store, &run.id)
            .iter()
            .filter(|event| event.detail["event"] == "terminal_status")
            .count(),
        2,
        "each committed outcome appends its own terminal event"
    );
}

/// The observer contract is unchanged by supersession: the superseding record
/// carries this attempt's own snapshot, so the observer is armed exactly once
/// for the outcome that is actually committed.
#[tokio::test]
async fn an_observer_eligible_supersession_arms_the_observer_once() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Accepted);

    finalize_run_status(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        RunStatus::Paused,
        "paused mid run",
        None,
    )
    .unwrap();
    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        Some(snapshot(temp.path())),
        &summary(WorkflowV2Status::Accepted),
        &v2_store,
        None,
        None,
    )
    .await
    .expect("a paused run may decide a completing status");

    let record = read_finalization(&store, &run.id);
    assert_eq!(record.terminal_v2_status, Some(WorkflowV2Status::Accepted));
    assert!(record.observer_snapshot.is_some());
    assert_eq!(record.observer_state, Some(RunEndObserverStateV1::Pending));
}
