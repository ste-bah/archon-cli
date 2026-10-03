//! Issue-256: `restart-task` is a run-control write. It takes the run lock and
//! moves the generation on, so a dispatcher that chose a reuse candidate
//! before the restart cannot write that candidate back over the restart: its
//! generation check fails, and the slot stays invalidated. Every check reads
//! the files back.

#[path = "support/restart_run.rs"]
mod restart_run;

use std::sync::mpsc;
use std::time::Duration;

use archon_workflow::v2::restart::restart_generated_v2_task;
use archon_workflow::{WorkflowError, WorkflowStore, WorkflowV2CallRecord, WorkflowV2ResultStore};
use restart_run::{accepted, agent_call, generated_run, interrupted, slot, v2_store};

const REASON: &str = "restart-task:T-A";

/// What the live host's history restore does (`restore_reused_record`): under
/// the run lock, and only while the dispatching generation still owns the
/// run, the reused record goes back into the call's slot.
fn restore_as_dispatcher(
    store: &WorkflowStore,
    run_id: &str,
    v2: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
    generation: u64,
) -> archon_workflow::WorkflowResult<()> {
    store.with_run_lock(run_id, |locked| {
        let current = locked.load_state(run_id)?.generation;
        if current != generation {
            return Err(WorkflowError::ControlCancelled(format!(
                "generation {generation} cannot restore; current generation is {current}"
            )));
        }
        v2.restore_call_record(record)
    })
}

#[test]
fn an_in_flight_history_restore_cannot_undo_a_restart_task() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &["author-a"]);
    let v2 = v2_store(&store, &run);
    v2.save_call_record(&accepted("author-a", "T-A")).unwrap();
    v2.save_call_record(&interrupted("author-a")).unwrap();

    // The dispatcher reads its generation and selects the history candidate.
    let generation = store.load_state(&run.id).unwrap().generation;
    let candidate = v2
        .call_record_for_reuse(&agent_call("author-a"), "in-author-a")
        .unwrap()
        .expect("history candidate");
    assert!(candidate.from_history);

    // The operator restarts the task before the dispatcher restores.
    restart_generated_v2_task(&store, &run, "T-A")
        .unwrap()
        .expect("generated run");
    assert!(
        store.load_state(&run.id).unwrap().generation > generation,
        "restart-task must move the generation on"
    );

    let restored = restore_as_dispatcher(&store, &run.id, &v2, &candidate.record, generation);
    assert!(
        matches!(restored, Err(WorkflowError::ControlCancelled(_))),
        "the stale dispatcher must fail its generation check: {restored:?}"
    );
    // Read back: the slot is still the invalidated interrupted attempt, and
    // nothing in the history answers the resume.
    let held = slot(&v2, "author-a").expect("slot");
    assert_eq!(held.invalidated_by.as_deref(), Some(REASON));
    assert_eq!(held.attempt, 2);
    assert!(
        v2.last_accepted_call_record("author-a", "in-author-a")
            .unwrap()
            .is_none()
    );
}

#[test]
fn restart_task_waits_for_the_run_lock() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &["author-a"]);
    let v2 = v2_store(&store, &run);
    v2.save_call_record(&accepted("author-a", "T-A")).unwrap();
    let generation = store.load_state(&run.id).unwrap().generation;

    let (done_tx, done_rx) = mpsc::channel();
    store
        .with_run_lock(&run.id, |_| {
            let (store, run) = (store.clone(), run.clone());
            std::thread::spawn(move || {
                let result = restart_generated_v2_task(&store, &run, "T-A").map(|_| ());
                done_tx.send(result).unwrap();
            });
            // While another holder has the lock, the restart writes nothing.
            assert!(
                done_rx.recv_timeout(Duration::from_millis(400)).is_err(),
                "restart-task ran while the run lock was held"
            );
            assert!(slot(&v2, "author-a").unwrap().invalidated_by.is_none());
            Ok(())
        })
        .unwrap();

    done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("restart finishes once the lock is free")
        .unwrap();
    assert_eq!(
        slot(&v2, "author-a").unwrap().invalidated_by.as_deref(),
        Some(REASON)
    );
    assert_eq!(
        store.load_state(&run.id).unwrap().generation,
        generation + 1
    );
}
