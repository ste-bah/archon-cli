//! Issue 326: the round the run end's re-entered acceptance stage writes
//! can be damaged, or unreadable, by the time the reopen reads it back. The
//! Issue 313 rule holds there too: never an error, never a verdict rebuilt
//! from another record. The damage is quarantined with evidence and the run
//! pauses; the resume runs the stage again and heals it.

use super::*;
use archon_workflow::{LifecycleAction, LifecycleController};

/// The round the re-entered stage writes after the in-run round.
const REENTERED: &str = "v2/acceptance/round-01/attempt-02.json";

/// Not JSON, and JSON of the wrong shape.
const DAMAGE: [&str; 2] = ["{\"round\":", r#"{"round":"one","attempt":2,"checks":{}}"#];

fn status(fixture: &Fixture) -> RunStatus {
    fixture.store.load_state(&fixture.run_id).unwrap().status
}

/// The in-run round passes, then the stage runs again as the reopen runs
/// it; returns where the re-entered round was written.
async fn reentered_round_written(fixture: &Fixture) -> std::path::PathBuf {
    in_run_round(fixture).await;
    in_run_round(fixture).await;
    let path = fixture.store.run_dir(&fixture.run_id).join(REENTERED);
    assert!(path.is_file(), "the re-entered round is written");
    path
}

fn read_back(fixture: &Fixture) -> WorkflowResult<String> {
    super::super::reopen::reentered_round(&fixture.store, &fixture.run_id, REENTERED)
        .map(|(record, _)| format!("round {} attempt {}", record.round, record.attempt))
}

/// The files of the re-entered round's quarantine, by name.
fn quarantine(fixture: &Fixture) -> Vec<std::path::PathBuf> {
    let dir = (fixture.store.run_dir(&fixture.run_id)).join("v2/acceptance/round-01/quarantine");
    std::fs::read_dir(dir)
        .map(|listing| listing.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default()
}

/// A damaged re-entered round pauses the run with evidence (its bytes and
/// an evidence file in quarantine, a quarantine event, a pause event naming
/// it). The resume runs the stage again: the reopen path writes a new round
/// and the run completes on it.
#[tokio::test]
async fn a_damaged_reentered_round_pauses_with_evidence_and_the_resume_heals() {
    for damage in DAMAGE {
        let fixture = fixture();
        let path = reentered_round_written(&fixture).await;
        std::fs::write(&path, damage).unwrap();

        let got = read_back(&fixture);

        let Err(WorkflowError::ControlPaused(message)) = &got else {
            panic!("{damage}: a damaged round pauses the run: {got:?}");
        };
        assert!(
            message.contains(REENTERED) && message.contains("quarantined"),
            "{damage}: {message}"
        );
        assert_eq!(status(&fixture), RunStatus::Paused, "{damage}");
        assert!(!path.exists(), "{damage}: the damaged record is moved");
        let kept = quarantine(&fixture);
        let damaged = kept
            .iter()
            .find(|p| p.to_string_lossy().ends_with(".damaged"))
            .unwrap_or_else(|| panic!("{damage}: no quarantined bytes in {kept:?}"));
        assert_eq!(std::fs::read_to_string(damaged).unwrap(), damage);
        assert!(
            kept.iter()
                .any(|p| p.to_string_lossy().ends_with(".evidence.json")),
            "{damage}: {kept:?}"
        );
        let labels = labels(&fixture);
        for label in ["acceptance_record_quarantined", "acceptance_gate_pause"] {
            assert!(labels.iter().any(|l| l == label), "{damage}: {labels:?}");
        }

        LifecycleController::new(fixture.store.clone())
            .apply(&fixture.run_id, LifecycleAction::Resume)
            .unwrap();
        let summary = in_run_round(&fixture).await;
        let finalized = finalize_with(&fixture, &scripted(&fixture, 1), summary).await;
        assert_eq!(finalized.status, WorkflowV2Status::Accepted, "{damage}");
        assert_eq!(status(&fixture), RunStatus::Completed, "{damage}");
        let gate = record(&fixture).acceptance_gate.expect("the new round");
        assert_eq!(
            gate.record_path, "v2/acceptance/round-01/attempt-04.json",
            "{damage}: judged on the round the resumed reopen wrote"
        );
    }
}

/// The file system will not hand the re-entered round over (a directory
/// stands in its place): the run pauses naming it, and nothing is moved.
#[tokio::test]
async fn an_unreadable_reentered_round_pauses_the_run() {
    let fixture = fixture();
    let path = reentered_round_written(&fixture).await;
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();

    let got = read_back(&fixture);

    let Err(WorkflowError::ControlPaused(message)) = &got else {
        panic!("an unreadable round pauses the run: {got:?}");
    };
    assert!(
        message.contains(REENTERED) && message.contains("cannot be read"),
        "{message}"
    );
    assert_eq!(status(&fixture), RunStatus::Paused);
    assert!(path.is_dir(), "nothing is moved");
    assert!(quarantine(&fixture).is_empty());
}
