//! Issue 316: a run end of an executor fenced by its launch generation
//! (`expected_generation: Some`) pauses after a mid-run edit that kept the
//! executor, never ends the run Cancelled. The edit (a restart of a stage
//! or item, a force-accept) moves the generation on; the pauses read the
//! generation while the executor owns the run, never the launch one.

use super::*;

/// What a lifecycle edit of a running run does (`lifecycle_a`): the
/// executor kept, the generation moved on. Returns the launch generation.
fn edit_keeping_the_executor(fixture: &Fixture) -> u64 {
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    let launch = run.generation;
    run.executor_generation = Some(launch);
    run.generation = launch + 1;
    fixture.store.save_state(&run).unwrap();
    launch
}

async fn finalize_owned(
    fixture: &Fixture,
    observer: &Scripted,
    summary: WorkflowV2ScriptSummary,
    launch: u64,
) -> anyhow::Result<WorkflowV2ScriptSummary> {
    finalize_run_observed(
        &fixture.store,
        &fixture.run_id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        Some(snapshot(fixture)),
        summary,
        &fixture.v2_store,
        observer,
        Some((&fixture.runtime, None, Some(&fixture.universe))),
        Some(launch),
    )
    .await
}

fn assert_paused(fixture: &Fixture, outcome: &anyhow::Result<WorkflowV2ScriptSummary>) -> String {
    let error = outcome.as_ref().expect_err("a stall pauses the run");
    let status = fixture.store.load_state(&fixture.run_id).unwrap().status;
    assert_eq!(status, RunStatus::Paused, "never Cancelled: {error:#}");
    match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<WorkflowError>())
    {
        Some(WorkflowError::ControlPaused(message)) => message.clone(),
        _ => panic!("a pause, never a cancel: {error:#}"),
    }
}

/// The finalizer's own no-progress pause after the edit: Paused.
#[tokio::test]
async fn the_finalizer_stall_after_an_edit_that_kept_the_executor_pauses() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    let launch = edit_keeping_the_executor(&fixture);

    let outcome = finalize_owned(&fixture, &scripted(&fixture, usize::MAX), summary, launch).await;

    let message = assert_paused(&fixture, &outcome);
    assert!(message.contains("made no progress"), "{message}");
}

/// The re-entered round's own pause after the edit (its history lost a
/// record of unknown state): Paused, under the generation read when it was
/// re-entered.
#[tokio::test]
async fn a_reentered_round_pause_after_an_edit_that_kept_the_executor_pauses() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let lost = archon_workflow::v2::acceptance_stage::round_dir(&run_dir, 0);
    std::fs::create_dir_all(&lost).unwrap();
    std::fs::write(lost.join("attempt-01.json"), "{").unwrap();
    std::fs::remove_file(run_dir.join("v2/acceptance/progress-ledger.json")).unwrap();
    let launch = edit_keeping_the_executor(&fixture);

    let outcome = finalize_owned(&fixture, &scripted(&fixture, 1), summary, launch).await;

    let message = assert_paused(&fixture, &outcome);
    assert!(message.contains("quarantine"), "{message}");
}

/// A resume replaced the executor before the run end: its pause is refused
/// and it writes nothing; the newer owner keeps the run.
#[tokio::test]
async fn a_replaced_executor_run_end_changes_nothing() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    let launch = run.generation;
    run.generation = launch + 2;
    run.executor_generation = Some(launch + 2);
    fixture.store.save_state(&run).unwrap();

    let outcome = finalize_owned(&fixture, &scripted(&fixture, usize::MAX), summary, launch).await;

    assert!(outcome.is_err(), "the stale executor finalizes nothing");
    let after = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(after.status, run.status, "the newer owner keeps the run");
}
