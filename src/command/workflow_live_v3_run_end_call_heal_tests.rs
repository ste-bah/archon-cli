//! Issue 313: the acceptance call's own record can be damaged by the time
//! the run ends. Not JSON and the wrong shape are the same damage: never an
//! error that fails the run, never "the stage recorded no round". The record
//! is rebuilt from its round record; when that is lost too, the run pauses,
//! and the resume heals it.

use super::*;
use archon_workflow::{LifecycleAction, LifecycleController};

const BOUND: &str = "v2/acceptance/round-01/attempt-01.json";

/// Not JSON, and JSON carrying every key the store looks for, wrongly typed.
const DAMAGE: [&str; 2] = [
    "{\"call\":",
    r#"{"call":{"id":"x"},"attempt":"one","input_hash":1,"status":"accepted","result":{}}"#,
];

fn damage_call(fixture: &Fixture, summary: &WorkflowV2ScriptSummary, damage: &str) {
    std::fs::write(fixture.v2_store.result_path(&summary.calls[0].id), damage).unwrap();
}

fn status(fixture: &Fixture) -> RunStatus {
    fixture.store.load_state(&fixture.run_id).unwrap().status
}

fn paused_message(error: &anyhow::Error) -> Option<String> {
    match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<WorkflowError>())
    {
        Some(WorkflowError::ControlPaused(message)) => Some(message.clone()),
        _ => None,
    }
}

/// The run end: the record is rebuilt from the round it names, the damaged
/// bytes are quarantined with an event, and the run ends on its round.
#[tokio::test]
async fn a_damaged_acceptance_call_record_is_rebuilt_from_its_round_and_the_run_ends() {
    for damage in DAMAGE {
        let fixture = fixture();
        let observer = scripted(&fixture, 0);
        let summary = in_run_round(&fixture).await;
        damage_call(&fixture, &summary, damage);

        let finalized = try_finalize_with(&fixture, &observer, summary).await;

        let finalized = finalized.unwrap_or_else(|error| panic!("{damage}: {error:#}"));
        assert_eq!(finalized.status, WorkflowV2Status::Accepted, "{damage}");
        assert_eq!(status(&fixture), RunStatus::Completed, "{damage}");
        let gate = record(&fixture).acceptance_gate;
        let gate = gate.unwrap_or_else(|| panic!("{damage}: the gate is dropped"));
        assert_eq!(gate.record_path, BOUND, "{damage}");
        assert!(
            labels(&fixture)
                .iter()
                .any(|l| l == "acceptance_call_record_quarantined"),
            "{damage}: {:?}",
            labels(&fixture)
        );
    }
}

/// Inside the runner (the authored outcome): a damaged call record gives
/// the outcome its whole copy gives, never an error the run fails on.
#[tokio::test]
async fn the_authored_outcome_reads_a_damaged_acceptance_call_as_its_round() {
    let outcome = |fixture: &Fixture, summary| {
        apply_authored_run_outcome(
            &fixture.store,
            &fixture.run_id,
            &fixture.v2_store,
            Some(&fixture.universe),
            Some(fixture.repo.path()),
            true,
            summary,
        )
        .map(|summary| {
            let next = summary.next_action.unwrap_or_default();
            (summary.status, next.replace(&fixture.run_id, "<run>"))
        })
    };
    let whole = fixture();
    let summary = in_run_round(&whole).await;
    let expected = outcome(&whole, summary).expect("a whole record");
    for damage in DAMAGE {
        let fixture = fixture();
        let summary = in_run_round(&fixture).await;
        damage_call(&fixture, &summary, damage);

        let got = outcome(&fixture, summary);

        let got = got.unwrap_or_else(|error| panic!("{damage}: {error}"));
        assert_eq!(got, expected, "{damage}");
    }
}

/// Both copies lost: the run pauses naming the call record, never Failed
/// and never Running; the resume runs the stage again and the run ends on
/// the new round.
#[tokio::test]
async fn a_damaged_call_record_with_no_whole_round_pauses_and_the_resume_heals() {
    for damage in DAMAGE {
        let fixture = fixture();
        let summary = in_run_round(&fixture).await;
        damage_call(&fixture, &summary, damage);
        let bound = fixture.store.run_dir(&fixture.run_id).join(BOUND);
        std::fs::write(bound, "{\"round\":").unwrap();

        let error = try_finalize_with(&fixture, &scripted(&fixture, 0), summary)
            .await
            .expect_err("no whole copy: the run pauses");

        let message = paused_message(&error).unwrap_or_else(|| panic!("{damage}: {error:#}"));
        assert!(message.contains("acceptance call record"), "{message}");
        assert_eq!(status(&fixture), RunStatus::Paused, "{damage}");
        LifecycleController::new(fixture.store.clone())
            .apply(&fixture.run_id, LifecycleAction::Resume)
            .unwrap();
        let summary = in_run_round(&fixture).await;
        let finalized = finalize_with(&fixture, &scripted(&fixture, 0), summary).await;
        assert_eq!(finalized.status, WorkflowV2Status::Accepted, "{damage}");
        assert_eq!(status(&fixture), RunStatus::Completed, "{damage}");
        let gate = record(&fixture)
            .acceptance_gate
            .expect("the new round's gate");
        assert_eq!(gate.record_path, "v2/acceptance/round-01/attempt-02.json");
    }
}

/// The file system will not hand the call record over (here: a directory
/// stands in the slot): the run pauses naming it, never Failed.
#[tokio::test]
async fn an_unreadable_acceptance_call_record_pauses_the_run() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    let slot = fixture.v2_store.result_path(&summary.calls[0].id);
    std::fs::remove_file(&slot).unwrap();
    std::fs::create_dir(&slot).unwrap();

    let error = try_finalize_with(&fixture, &scripted(&fixture, 0), summary)
        .await
        .expect_err("an unreadable record pauses the run");

    let message = paused_message(&error).unwrap_or_else(|| panic!("{error:#}"));
    assert!(message.contains("cannot be read"), "{message}");
    assert_eq!(status(&fixture), RunStatus::Paused);
}
