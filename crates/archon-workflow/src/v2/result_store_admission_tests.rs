//! Round 4 (Issue 291): a rerun after a superseded dispatch of a generated
//! run is a new admission, with its own order and start time.
use super::*;
use crate::WorkflowV2HostMethod;

fn store() -> (tempfile::TempDir, WorkflowV2ResultStore) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("workflows/run-1/v2"));
    (temp, store)
}

fn running(id: &str, attempt: u32, input_hash: &str) -> WorkflowV2CallRecord {
    WorkflowV2CallRecord::new(
        "run-1",
        WorkflowV2HostCall {
            id: id.into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        attempt,
        input_hash.into(),
        WorkflowV2Result {
            status: WorkflowV2Status::Running,
            ..WorkflowV2Result::default()
        },
        Vec::new(),
    )
}

/// Admit `id` the way a dispatch does: next attempt, then the admission.
fn admit(store: &WorkflowV2ResultStore, id: &str, input_hash: &str) -> WorkflowV2CallRecord {
    let attempt = store.next_dispatch_attempt(id, input_hash).unwrap();
    let mut record = running(id, attempt, input_hash);
    // Distinct wall-clock starts, even on a coarse clock.
    std::thread::sleep(std::time::Duration::from_millis(5));
    record.started_at = chrono::Utc::now().to_rfc3339();
    store.save_call_admission(&record).unwrap();
    store.record_with_admission(&record).unwrap()
}

#[test]
fn a_superseded_dispatch_rerun_gets_a_fresh_admission() {
    let (_temp, store) = store();
    let superseded = admit(&store, "call", "input");
    let sibling = admit(&store, "other", "input");
    // The superseded dispatch saved no call record; its rerun is new.
    let rerun = admit(&store, "call", "input");
    assert_eq!(superseded.attempt, 1);
    assert_eq!(rerun.attempt, 2, "the admitted attempt is used");
    assert!(rerun.admission_sequence > sibling.admission_sequence);
    assert_ne!(rerun.started_at, superseded.started_at);
    // The superseded admission stays as evidence, unchanged.
    let kept = store
        .record_with_admission(&running("call", 1, "input"))
        .unwrap();
    assert_eq!(kept.admission_sequence, superseded.admission_sequence);
    assert_eq!(kept.started_at, superseded.started_at);
}

#[test]
fn repeated_supersession_keeps_moving_the_attempt_on() {
    let (_temp, store) = store();
    let attempts = (0..3)
        .map(|_| admit(&store, "call", "input"))
        .map(|record| (record.attempt, record.admission_sequence))
        .collect::<Vec<_>>();
    assert_eq!(
        attempts,
        vec![(1, Some(1)), (2, Some(2)), (3, Some(3))],
        "each rerun is its own admission"
    );
}

#[test]
fn a_new_input_or_a_fresh_call_starts_at_the_recorded_attempt() {
    let (_temp, store) = store();
    assert_eq!(store.next_dispatch_attempt("call", "input").unwrap(), 1);
    admit(&store, "call", "input");
    assert_eq!(
        store.next_dispatch_attempt("call", "changed").unwrap(),
        1,
        "a different input is a different admission identity"
    );
    assert_eq!(store.next_dispatch_attempt("fresh", "input").unwrap(), 1);
}

#[test]
fn a_recorded_attempt_still_moves_on_as_before() {
    let (_temp, store) = store();
    let admitted = admit(&store, "call", "input");
    let mut done = admitted.clone();
    done.result.status = WorkflowV2Status::Accepted;
    done.status = WorkflowV2Status::Accepted;
    store.save_call_record(&done).unwrap();
    assert_eq!(store.next_dispatch_attempt("call", "input").unwrap(), 2);
    // Completing the admitted attempt inherits its own admission.
    let completed = store.load_call_record("call").unwrap().unwrap();
    assert_eq!(completed.admission_sequence, admitted.admission_sequence);
}
