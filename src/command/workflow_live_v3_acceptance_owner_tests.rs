//! Issue 316: the round's owner never pauses because another writer took
//! its attempt number. Two rounds of one acceptance round can be in flight
//! at once -- one of an executor a pause and resume replaced, still
//! finishing, beside the resumed owner's -- and both chose their attempt
//! when they started. The obsolete one is refused by its generation and
//! lands nothing; when any other writer took the number first, the owner's
//! record takes the next free one.

use super::*;
use archon_workflow::LifecycleAction;
use archon_workflow::v2::acceptance_stage::{AcceptanceRoundRecordV1, attempt_file_name};

/// A round whose one check failed and is owned: never final, never clean.
fn failing(fixture: &Fixture, round: u32, attempt: u32, check: &str) -> AcceptanceRoundRecordV1 {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "run_id": fixture.run_id,
        "call_id": format!("acceptance-contract-run-{round}"),
        "round": round,
        "attempt": attempt,
        "max_rounds": 3,
        "contract_present": true,
        "checks": [{
            "check_id": check,
            "criterion": "criterion",
            "kind": "command",
            "status": "failed",
            "owning_tasks": ["TASK-F-002"],
        }],
        "final_round": false,
    }))
    .unwrap()
}

fn generation(fixture: &Fixture) -> u64 {
    fixture
        .store
        .load_state(&fixture.run_id)
        .unwrap()
        .generation
}

/// An operator pause and resume: a newer executor owns the run, and the
/// generation the old one started under is obsolete. Returns (old, new).
fn pause_and_resume(fixture: &Fixture) -> (u64, u64) {
    let old = generation(fixture);
    let lifecycle = archon_workflow::LifecycleController::new(fixture.store.clone());
    lifecycle
        .apply(&fixture.run_id, LifecycleAction::Pause)
        .unwrap();
    lifecycle
        .apply(&fixture.run_id, LifecycleAction::Resume)
        .unwrap();
    (old, generation(fixture))
}

fn record(
    fixture: &Fixture,
    generation: u64,
    record: &mut AcceptanceRoundRecordV1,
) -> Result<super::super::ledger::Decided, WorkflowError> {
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    super::super::ledger::record_and_decide(
        &fixture.store,
        &fixture.run_id,
        generation,
        &run_dir,
        record,
        true,
    )
}

fn on_disk(fixture: &Fixture, attempt: u32) -> Option<AcceptanceRoundRecordV1> {
    let dir = round_dir(&fixture.store.run_dir(&fixture.run_id), 1);
    let bytes = std::fs::read(dir.join(attempt_file_name(attempt))).ok()?;
    Some(serde_json::from_slice(&bytes).unwrap())
}

/// The run was never paused by a round: not paused, and no pause evidence.
fn assert_not_paused(fixture: &Fixture) {
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_ne!(run.status, RunStatus::Paused, "the owner never pauses");
    let events =
        std::fs::read_to_string(fixture.store.events_path(&fixture.run_id)).unwrap_or_default();
    assert!(
        !events.contains("acceptance_history_pause"),
        "no round paused the run: {events}"
    );
}

/// The obsolete writer reaches the lock first: it is refused by its
/// generation and lands nothing, so the owner records its own number.
#[test]
fn an_obsolete_writer_first_lands_nothing_and_the_owner_records() {
    let fixture = fixture(true);
    let (old, new) = pause_and_resume(&fixture);

    let obsolete = record(&fixture, old, &mut failing(&fixture, 1, 1, "REQ-9"));
    let mut owned = failing(&fixture, 1, 1, "REQ-2");
    let current = record(&fixture, new, &mut owned);

    assert!(
        matches!(obsolete, Err(WorkflowError::ControlCancelled(_))),
        "an obsolete generation stops: {:?}",
        obsolete.as_ref().map(|decided| &decided.path)
    );
    let current = current.expect("the owner records");
    assert!(current.path.ends_with(attempt_file_name(1)));
    assert_eq!(owned.attempt, 1);
    assert_eq!(on_disk(&fixture, 1).unwrap().checks[0].check_id, "REQ-2");
    assert!(
        on_disk(&fixture, 2).is_none(),
        "the obsolete round left nothing"
    );
    assert_not_paused(&fixture);
}

/// The owner reaches the lock first: the obsolete writer that comes after
/// it is refused by its generation, never pauses the run, and leaves the
/// owner's record alone.
#[test]
fn the_owner_first_then_an_obsolete_writer_changes_nothing() {
    let fixture = fixture(true);
    let (old, new) = pause_and_resume(&fixture);

    record(&fixture, new, &mut failing(&fixture, 1, 1, "REQ-2")).expect("the owner records");
    let obsolete = record(&fixture, old, &mut failing(&fixture, 1, 1, "REQ-9"));

    assert!(
        matches!(obsolete, Err(WorkflowError::ControlCancelled(_))),
        "an obsolete generation stops: {:?}",
        obsolete.as_ref().map(|decided| &decided.path)
    );
    assert_eq!(on_disk(&fixture, 1).unwrap().checks[0].check_id, "REQ-2");
    assert!(
        on_disk(&fixture, 2).is_none(),
        "the obsolete round left nothing"
    );
    assert_not_paused(&fixture);
}

/// Two writers of the current generation chose one attempt: the second
/// takes the next free number instead of pausing, and decides on the
/// history that holds the first one's record (the same failing set again
/// is a revisit, and escalates).
#[test]
fn a_second_owner_writer_takes_the_next_attempt_and_counts_the_first() {
    let fixture = fixture(true);
    let owner = generation(&fixture);

    let first = record(&fixture, owner, &mut failing(&fixture, 1, 1, "REQ-2")).unwrap();
    let mut late = failing(&fixture, 1, 1, "REQ-2");
    let second = record(&fixture, owner, &mut late).expect("never refused for a number");

    assert!(first.path.ends_with(attempt_file_name(1)));
    assert!(second.path.ends_with(attempt_file_name(2)));
    assert_eq!(late.attempt, 2, "the record says the number it landed as");
    assert_eq!(on_disk(&fixture, 2).unwrap().attempt, 2);
    assert_eq!(
        second.decision.stalled_rounds, 1,
        "the first record counted"
    );
    assert!(second.decision.escalate);
    assert_not_paused(&fixture);
}
