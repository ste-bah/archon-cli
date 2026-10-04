//! Issue 262, round 9 (P1): the acceptance record the final gate is bound
//! to can be damaged, or already quarantined, by the time the run ends.
//! The gate never errors over a run left Running: it is rebuilt from the
//! acceptance call's own result (a whole second copy of the round), and
//! when no whole copy survives the run PAUSES with the evidence.

use super::*;
use archon_workflow::v2::acceptance_stage::progress::ProgressLedger;

const BOUND: &str = "v2/acceptance/round-01/attempt-01.json";

fn bound(fixture: &Fixture) -> std::path::PathBuf {
    fixture.store.run_dir(&fixture.run_id).join(BOUND)
}

fn status(fixture: &Fixture) -> RunStatus {
    fixture.store.load_state(&fixture.run_id).unwrap().status
}

/// The run committed `Completed` on the in-run round's passing gate.
fn assert_completed_on_the_bound_round(fixture: &Fixture, finalized: &WorkflowV2ScriptSummary) {
    assert_eq!(
        finalized.status,
        WorkflowV2Status::Accepted,
        "{finalized:?}"
    );
    assert_eq!(status(fixture), RunStatus::Completed);
    let gate = record(fixture).acceptance_gate.expect("the gate survives");
    assert_eq!(gate.record_path, BOUND);
    assert!(gate.failing_check_ids.is_empty(), "{gate:?}");
}

/// The bound record is corrupted after acceptance finished: the gate is
/// rebuilt from the call's result and the run ends on it, with the rebuild
/// recorded; never an EOF error over a run left Running.
#[tokio::test]
async fn a_corrupt_bound_record_is_rebuilt_from_its_call_and_the_run_ends() {
    let fixture = fixture();
    let observer = scripted(&fixture, 0);
    let summary = in_run_round(&fixture).await;
    std::fs::write(bound(&fixture), "{\"round\":").unwrap();

    let finalized = try_finalize_with(&fixture, &observer, summary).await;

    let finalized = finalized.expect("a damaged bound record heals or pauses, never errors");
    assert_completed_on_the_bound_round(&fixture, &finalized);
    assert!(
        labels(&fixture)
            .iter()
            .any(|l| l == "acceptance_gate_rebuilt"),
        "{:?}",
        labels(&fixture)
    );
}

/// Healing already quarantined the bound record (its ledger copy holds
/// only the failing ids): the gate is still rebuilt from the call's result,
/// never dropped.
#[tokio::test]
async fn a_quarantined_bound_record_keeps_its_gate() {
    let fixture = fixture();
    let observer = scripted(&fixture, 0);
    let summary = in_run_round(&fixture).await;
    std::fs::write(bound(&fixture), "{\"round\":").unwrap();
    let healed = ProgressLedger::load_healing(&fixture.store.run_dir(&fixture.run_id)).unwrap();
    assert_eq!(healed.quarantined.len(), 1, "{healed:?}");
    assert!(!bound(&fixture).exists());

    let finalized = try_finalize_with(&fixture, &observer, summary).await;

    let finalized = finalized.expect("a quarantined bound record heals or pauses");
    assert_completed_on_the_bound_round(&fixture, &finalized);
}

/// The bound record is damaged and the call's result is no whole copy of
/// the round either: the run pauses with the reason, never Running.
#[tokio::test]
async fn a_damaged_bound_record_with_no_whole_copy_pauses_the_run() {
    let fixture = fixture();
    let observer = scripted(&fixture, 0);
    let summary = in_run_round(&fixture).await;
    std::fs::write(bound(&fixture), "{\"round\":").unwrap();
    let call_id = &summary.calls[0].id;
    let mut call = fixture.v2_store.load_call_record(call_id).unwrap().unwrap();
    call.result.data = serde_json::json!({ "record_path": BOUND });
    fixture.v2_store.save_call_record(&call).unwrap();

    let error = try_finalize_with(&fixture, &observer, summary)
        .await
        .expect_err("no whole copy: the run pauses");

    let paused = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<WorkflowError>());
    assert!(
        matches!(paused, Some(WorkflowError::ControlPaused(message)) if message.contains(BOUND)),
        "{error:#}"
    );
    assert_eq!(status(&fixture), RunStatus::Paused);
}

/// A rebuilt gate is never a false green: the bound round failed REQ-1,
/// its record is corrupted, and the run still ends `NeedsReview` on REQ-1.
#[tokio::test]
async fn a_rebuilt_gate_keeps_the_failing_checks() {
    let fixture = fixture();
    std::fs::remove_file(fixture.repo.path().join("present")).unwrap();
    let summary = in_run_round(&fixture).await;
    std::fs::write(bound(&fixture), "{\"round\":").unwrap();

    let finalized = finalize_with(&fixture, &scripted(&fixture, 0), summary).await;

    assert_eq!(finalized.status, WorkflowV2Status::NeedsReview);
    assert_eq!(status(&fixture), RunStatus::NeedsReview);
    let gate = record(&fixture).acceptance_gate.expect("the gate survives");
    assert_eq!(gate.record_path, BOUND);
    assert_eq!(gate.failing_check_ids, vec!["REQ-1".to_string()]);
    assert!(gate.contract_present);
}
