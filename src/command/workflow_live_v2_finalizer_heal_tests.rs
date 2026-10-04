//! Progress is a failing set the run never reached (round 3, decision A);
//! a revisit pauses the run, never ends it, and the sets reached are kept
//! across a pause (Issue 262).

use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::{
    RunEndObserverOutcomeV1, RunStatus, WorkflowError, WorkflowResult, WorkflowRunKind,
    WorkflowStore, WorkflowV2ResultStore, WorkflowV2Status,
};

use super::super::super::workflow_live_v2_script::WorkflowV2ScriptSummary;
use super::super::super::workflow_run_finalizer_tests::{seed_call, snapshot, spec, summary};
use super::super::{RunEndObserverContext, WorkflowRunEndObserver, finalize_summary_with_gate};
use super::state::masked;
use super::{Reopened, RunEndReopen};

#[test]
fn masking_hides_minted_ids_and_counts_but_keeps_the_failure() {
    let first = masked(
        "native observation failed in /tmp/evidence-4f1c2a9b8d7e6f50a1b2c3d4e5f60718 after 12 checks",
    );
    let second = masked(
        "native observation failed in /tmp/evidence-0a9b8c7d6e5f40312a3b4c5d6e7f8091 after 13 checks",
    );
    assert_eq!(first, second);
    assert!(first.contains("native observation failed"), "{first}");
    assert_ne!(
        masked("chain check unrecorded_change failed"),
        masked("chain check preimage_corrupt failed"),
        "a different failure is progress"
    );
}

/// B1: a hyphenated uuid, a timestamp and an evidence or scratch path a
/// retry mints are not progress; the failure text around them still is.
#[test]
fn masking_hides_hyphenated_ids_timestamps_and_evidence_paths() {
    let first = masked(
        "run 4f1c2a9b-8d7e-4f50-a1b2-c3d4e5f60718 at 2026-09-29T23:48:18.822015Z: teardown failed in /Volumes/x/archon-native-observations/evidence-7d1e/run/out.json and /var/scratch/wt-a1/target",
    );
    let second = masked(
        "run 0a9b8c7d-6e5f-4031-82a3-b4c5d6e7f809 at 2026-09-30T01:02:03Z: teardown failed in /Volumes/x/archon-native-observations/evidence-9f0c/run/out.json and /var/scratch/wt-b2/target",
    );
    assert_eq!(first, second);
    assert!(first.contains("teardown failed in"), "{first}");
    assert_ne!(first, masked("run x: teardown passed"));
}

/// Fails every observation with a reason no earlier one had, so progress
/// never stops re-entry: only the runaway guard can.
struct EverNew {
    from: usize,
    calls: AtomicUsize,
}

impl WorkflowRunEndObserver for EverNew {
    fn observe(&self, _: &RunEndObserverContext<'_>) -> WorkflowResult<RunEndObserverOutcomeV1> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            call <= 200,
            "re-entry did not stop after {call} observations"
        );
        Err(WorkflowError::StageFailed(format!(
            "probe {} failed",
            word(self.from + call)
        )))
    }
}

/// A distinct word per `n`, letters only: masking hides digits.
fn word(mut n: usize) -> String {
    let mut word = String::new();
    loop {
        word.push(char::from(b'a' + (n % 26) as u8));
        n /= 26;
        if n == 0 {
            return word;
        }
    }
}

/// Re-enters by returning the outcome unchanged; pauses on call `pause_at`.
pub(super) struct Reentry {
    pub(super) calls: AtomicUsize,
    pub(super) pause_at: usize,
}

#[async_trait::async_trait]
impl RunEndReopen for Reentry {
    async fn reopen(&self, summary: &WorkflowV2ScriptSummary) -> WorkflowResult<Option<Reopened>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == self.pause_at {
            return Err(WorkflowError::ControlPaused("operator pause".into()));
        }
        Ok(Some(Reopened {
            summary: summary.clone(),
            gate: None,
            record_path: None,
        }))
    }
}

async fn finalize(
    store: &WorkflowStore,
    run_id: &str,
    root: &std::path::Path,
    observer: &EverNew,
    reentry: &Reentry,
) -> WorkflowResult<WorkflowV2ScriptSummary> {
    finalize_with(store, run_id, root, observer, reentry).await
}

pub(super) async fn finalize_with(
    store: &WorkflowStore,
    run_id: &str,
    root: &std::path::Path,
    observer: &dyn WorkflowRunEndObserver,
    reentry: &dyn RunEndReopen,
) -> WorkflowResult<WorkflowV2ScriptSummary> {
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    finalize_summary_with_gate(
        store,
        run_id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        Some(snapshot(root)),
        &summary(WorkflowV2Status::Accepted),
        &v2_store,
        Some(observer),
        None,
        None,
        Some(reentry),
    )
    .await
}

pub(super) fn setup() -> (tempfile::TempDir, WorkflowStore, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Accepted);
    (temp, store, run.id)
}

/// Round 3 (decision A): progress is a failing set the run never reached,
/// never the failure's text. Observations that fail with ever-new errors but
/// no failing check reach the same (empty) failing set again: the first
/// revisit pauses the run, with the evidence, and never ends it.
#[tokio::test]
async fn changing_errors_without_a_new_failing_set_pause_at_the_first_revisit() {
    let (temp, store, run_id) = setup();
    let observer = EverNew {
        from: 0,
        calls: AtomicUsize::new(0),
    };
    let reentry = Reentry {
        calls: AtomicUsize::new(0),
        pause_at: 0,
    };
    let error = finalize(&store, &run_id, temp.path(), &observer, &reentry)
        .await
        .expect_err("a revisit pauses");
    let WorkflowError::ControlPaused(message) = &error else {
        panic!("a stopped re-entry pauses, never ends the run: {error:?}");
    };
    assert!(message.contains("made no progress"), "{message}");
    assert_eq!(reentry.calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.load_state(&run_id).unwrap().status, RunStatus::Paused);
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    assert!(
        events.contains("run_end_acceptance_observer_stall_pause"),
        "{events}"
    );
}

/// The failing sets reached are kept across a pause: a resumed finalization
/// that reaches one of them again pauses without re-entering.
#[tokio::test]
async fn the_failing_sets_reached_survive_a_pause() {
    let (temp, store, run_id) = setup();
    let first = EverNew {
        from: 0,
        calls: AtomicUsize::new(0),
    };
    let paused = Reentry {
        calls: AtomicUsize::new(0),
        pause_at: 1,
    };
    let error = finalize(&store, &run_id, temp.path(), &first, &paused)
        .await
        .expect_err("the operator pause stops finalization");
    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    let mut run = store.load_state(&run_id).unwrap();
    run.status = RunStatus::Running;
    store.save_state(&run).unwrap();

    let resumed = EverNew {
        from: 1000,
        calls: AtomicUsize::new(0),
    };
    let reentry = Reentry {
        calls: AtomicUsize::new(0),
        pause_at: 0,
    };
    let error = finalize(&store, &run_id, temp.path(), &resumed, &reentry)
        .await
        .expect_err("the set was reached before the pause");
    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    assert_eq!(reentry.calls.load(Ordering::SeqCst), 0);
}

#[path = "workflow_live_v2_finalizer_heal_progress_tests.rs"]
mod progress_tests;
