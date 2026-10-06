//! Issue 316 (round 3 of the review): an operator edit made in the window
//! between a run-end caller deciding to pause and the pause -- a restart of
//! a stage, which keeps the executor and moves the generation on -- never
//! ends the run Cancelled. Every such pause checks the executor with the
//! pause under the run lock. The edit is made by `LifecycleController::
//! apply_restart` from the hook every run-end pause passes just before it
//! takes the run lock.

use super::*;
use archon_workflow::v2::acceptance_stage::{AcceptanceRoundRecordV1, write_round_record};
use archon_workflow::{LifecycleAction, LifecycleController};

/// A stage the operator restarts in the window.
const OPERATOR_STAGE: &str = "operator-stage";

/// The run as its live executor holds it: running, with a stage an operator
/// can restart, its executor launched at the generation returned.
fn held_by_its_executor(fixture: &Fixture) -> u64 {
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Running;
    let stage = archon_workflow::run::StageState::pending(OPERATOR_STAGE);
    run.stages.insert(OPERATOR_STAGE.into(), stage);
    run.executor_generation = Some(run.generation);
    fixture.store.save_state(&run).unwrap();
    run.generation
}

/// The next run-end pause first lets `edit` act on the run.
fn in_the_window(fixture: &Fixture, edit: LifecycleAction) {
    let (store, run_id) = (fixture.store.clone(), fixture.run_id.clone());
    super::super::owned_pause::BEFORE_PAUSE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            LifecycleController::new(store)
                .apply_restart(&run_id, edit)
                .unwrap();
        }));
    });
}

fn restart() -> LifecycleAction {
    LifecycleAction::RestartStage(OPERATOR_STAGE.into())
}

async fn finalize_for(
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

/// Paused (never Cancelled), after the edit in the window was made.
fn assert_paused_after_the_edit(
    fixture: &Fixture,
    launch: u64,
    outcome: &anyhow::Result<WorkflowV2ScriptSummary>,
) -> String {
    let error = outcome.as_ref().expect_err("the run end pauses");
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused, "never Cancelled: {error:#}");
    assert_eq!(
        run.executor_generation,
        Some(launch),
        "the executor was kept"
    );
    assert_eq!(run.generation, launch + 2, "the edit, then the pause");
    let still_pending = super::super::owned_pause::BEFORE_PAUSE.with(|h| h.borrow().is_some());
    assert!(!still_pending, "the edit was made in the window");
    let events = std::fs::read_to_string(fixture.store.events_path(&fixture.run_id)).unwrap();
    assert!(
        !events.contains("run_end_refused_pause"),
        "the pause itself held, not the refused-pause net"
    );
    match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<WorkflowError>())
    {
        Some(WorkflowError::ControlPaused(message)) => message.clone(),
        _ => panic!("a pause, never a cancel: {error:#}"),
    }
}

/// The finalizer's own no-progress pause.
#[tokio::test]
async fn an_edit_before_the_finalizer_pause_still_pauses() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    let launch = held_by_its_executor(&fixture);
    in_the_window(&fixture, restart());

    let outcome = finalize_for(&fixture, &scripted(&fixture, usize::MAX), summary, launch).await;

    let message = assert_paused_after_the_edit(&fixture, launch, &outcome);
    assert!(message.contains("made no progress"), "{message}");
}

/// The re-entered round's history pause (a record of unknown state lost).
#[tokio::test]
async fn an_edit_before_the_reentered_round_history_pause_still_pauses() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let lost = archon_workflow::v2::acceptance_stage::round_dir(&run_dir, 0);
    std::fs::create_dir_all(&lost).unwrap();
    std::fs::write(lost.join("attempt-01.json"), "{").unwrap();
    std::fs::remove_file(run_dir.join("v2/acceptance/progress-ledger.json")).unwrap();
    let launch = held_by_its_executor(&fixture);
    in_the_window(&fixture, restart());

    let outcome = finalize_for(&fixture, &scripted(&fixture, 1), summary, launch).await;

    let message = assert_paused_after_the_edit(&fixture, launch, &outcome);
    assert!(message.contains("quarantine"), "{message}");
}

/// A record of round 0 failing REQ-1, written after the in-run round.
fn failing_req_1(fixture: &Fixture, attempt: u32) -> AcceptanceRoundRecordV1 {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "run_id": fixture.run_id,
        "call_id": "acceptance-contract-run-0",
        "round": 0,
        "attempt": attempt,
        "max_rounds": 3,
        "contract_present": true,
        "checks": [{
            "check_id": "REQ-1",
            "criterion": "criterion",
            "kind": "command",
            "status": "failed",
            "owning_tasks": ["TASK-H-001"],
        }],
        "final_round": false,
    }))
    .unwrap()
}

/// The re-entered round's stall pause: REQ-1 failing a third time in a row.
#[tokio::test]
async fn an_edit_before_the_reentered_round_stall_pause_still_pauses() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    for attempt in [1, 2] {
        write_round_record(&run_dir, &failing_req_1(&fixture, attempt)).unwrap();
    }
    std::fs::remove_file(fixture.repo.path().join("present")).unwrap();
    let launch = held_by_its_executor(&fixture);
    in_the_window(&fixture, restart());

    let outcome = finalize_for(&fixture, &scripted(&fixture, 1), summary, launch).await;

    let message = assert_paused_after_the_edit(&fixture, launch, &outcome);
    assert!(message.contains("made no progress"), "{message}");
}

/// A resume in the window replaced the executor: its pause is refused and
/// nothing changes; the newer owner keeps the run, neither paused nor
/// cancelled.
#[tokio::test]
async fn a_resume_before_the_finalizer_pause_changes_nothing() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    let launch = held_by_its_executor(&fixture);
    let (store, run_id) = (fixture.store.clone(), fixture.run_id.clone());
    super::super::owned_pause::BEFORE_PAUSE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let lifecycle = LifecycleController::new(store);
            lifecycle.apply(&run_id, LifecycleAction::Pause).unwrap();
            lifecycle.apply(&run_id, LifecycleAction::Resume).unwrap();
        }));
    });

    let outcome = finalize_for(&fixture, &scripted(&fixture, usize::MAX), summary, launch).await;

    assert!(outcome.is_err(), "the replaced executor finalizes nothing");
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(
        run.status,
        RunStatus::Running,
        "the newer owner keeps the run"
    );
    assert!(
        run.executor_generation > Some(launch),
        "a newer executor owns it"
    );
}
