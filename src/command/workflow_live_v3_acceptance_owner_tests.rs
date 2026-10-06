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
    // Review A3: the ledger copy is saved under the order lock, in record
    // order: it holds both records, as the numbers they landed as.
    use archon_workflow::v2::acceptance_stage::progress::{PROGRESS_LEDGER_FILE, ProgressLedger};
    let copy = (fixture.store.run_dir(&fixture.run_id))
        .join(archon_workflow::v2::acceptance_stage::ACCEPTANCE_RECORDS_DIR)
        .join(PROGRESS_LEDGER_FILE);
    let copy: ProgressLedger = serde_json::from_slice(&std::fs::read(copy).unwrap()).unwrap();
    let landed: Vec<(u32, u32)> = (copy.observed.iter())
        .map(|observed| (observed.round, observed.attempt))
        .collect();
    assert_eq!(landed, [(1, 1), (1, 2)]);
    assert_not_paused(&fixture);
}

/// Review B1: the run is resumed after the writer was checked and decided,
/// before its record lands. The owner check is made again with the landing
/// under the run lock, so the obsolete record never lands, and the new
/// owner records its own number without counting it.
#[test]
fn a_resume_between_the_decision_and_the_landing_lands_nothing() {
    let fixture = fixture(true);
    let old = generation(&fixture);
    let resumer = (fixture.store.clone(), fixture.run_id.clone());
    super::super::ledger::BEFORE_LAND.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let lifecycle = archon_workflow::LifecycleController::new(resumer.0);
            lifecycle.apply(&resumer.1, LifecycleAction::Pause).unwrap();
            lifecycle
                .apply(&resumer.1, LifecycleAction::Resume)
                .unwrap();
        }));
    });

    let obsolete = record(&fixture, old, &mut failing(&fixture, 1, 1, "REQ-2"));

    assert!(
        matches!(obsolete, Err(WorkflowError::ControlCancelled(_))),
        "the replaced executor lands nothing: {:?}",
        obsolete.as_ref().map(|decided| &decided.path)
    );
    assert!(on_disk(&fixture, 1).is_none(), "nothing landed");
    let new = generation(&fixture);
    assert!(new > old, "the hook resumed the run");
    let current = record(&fixture, new, &mut failing(&fixture, 1, 1, "REQ-2")).unwrap();
    assert!(current.path.ends_with(attempt_file_name(1)));
    assert_eq!(
        current.decision.stalled_rounds, 0,
        "no obsolete record counted"
    );
    assert_not_paused(&fixture);
}

/// Review B2: a round dispatched before a resume runs under the generation
/// it was dispatched at, never the newer owner's read when it starts: it
/// lands nothing and pauses nothing.
#[tokio::test]
async fn a_round_dispatched_before_a_resume_never_adopts_the_new_owner() {
    let fixture = fixture(true);
    let dispatched = generation(&fixture);
    pause_and_resume(&fixture);

    let error = run_at(&fixture, &execution(1, 3, &[]), dispatched).await;

    assert!(
        matches!(error, Err(WorkflowError::ControlCancelled(_))),
        "the replaced executor stops: {:?}",
        error.as_ref().map(|result| &result.status)
    );
    assert!(on_disk(&fixture, 1).is_none(), "nothing landed");
    assert_not_paused(&fixture);
}

struct NoLlm;

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for NoLlm {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        panic!("unexpected LLM request")
    }
}

/// The script host's dispatch (`execute_v2_live_call`, which the host calls
/// with the call generation it sampled while it owned the run) runs the
/// round under that generation. Dispatched before a resume, it lands
/// nothing; dispatched by the new owner, it records.
#[tokio::test]
async fn the_host_dispatch_runs_the_round_under_its_dispatch_generation() {
    use super::super::super::{LiveV2AgentClient, execute_v2_live_call};
    let fixture = fixture(true);
    let dispatched = generation(&fixture);
    pause_and_resume(&fixture);
    let (ui_sink, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        std::sync::Arc::new(NoLlm),
        ui_sink,
        vec![],
        fixture.run_id.clone(),
        None,
        None,
    );
    let v2 = archon_workflow::WorkflowV2ResultStore::new(
        fixture.store.run_dir(&fixture.run_id).join("v2"),
    );
    let dispatch = |generation| {
        execute_v2_live_call(
            "acceptance",
            &fixture.runtime,
            execution(1, 3, &[]),
            archon_workflow::WorkflowV2AgentAdapter::new(),
            &client,
            &v2,
            &fixture.store,
            &fixture.run_id,
            true,
            Some(&fixture.universe),
            None,
            false,
            generation,
        )
    };

    let stale = dispatch(dispatched).await;

    assert!(
        matches!(stale, Err(WorkflowError::ControlCancelled(_))),
        "the replaced executor's dispatch stops: {:?}",
        stale.as_ref().map(|result| &result.status)
    );
    assert!(on_disk(&fixture, 1).is_none(), "nothing landed");
    dispatch(generation(&fixture))
        .await
        .expect("the new owner's dispatch records");
    assert!(on_disk(&fixture, 1).is_some());
    assert_not_paused(&fixture);
}
