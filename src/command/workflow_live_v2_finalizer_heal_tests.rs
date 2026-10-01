//! Progress is judged on what changed, never on ids a retry mints; re-entry
//! is bounded by `REOPEN_LIMIT`, counted across a pause.

use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::{
    RunEndObserverOutcomeV1, RunStatus, WorkflowError, WorkflowResult, WorkflowRunKind,
    WorkflowStore, WorkflowV2ResultStore, WorkflowV2Status,
};

use super::super::super::workflow_live_v2_script::WorkflowV2ScriptSummary;
use super::super::super::workflow_run_finalizer_tests::{
    read_finalization, seed_call, snapshot, spec, summary,
};
use super::super::{RunEndObserverContext, WorkflowRunEndObserver, finalize_summary_with_gate};
use super::state::{REOPEN_LEDGER_PATH, ReopenLedger, masked};
use super::{REOPEN_LIMIT, Reopened, RunEndReopen};

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
/// never stops re-entry: only the limit can.
struct EverNew {
    from: usize,
    calls: AtomicUsize,
}

impl WorkflowRunEndObserver for EverNew {
    fn observe(&self, _: &RunEndObserverContext<'_>) -> WorkflowResult<RunEndObserverOutcomeV1> {
        const WORDS: [&str; 12] = [
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
            "juliet", "kilo", "lima",
        ];
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let word = WORDS
            .get(self.from + call)
            .unwrap_or_else(|| panic!("re-entry did not stop after {call} observations"));
        Err(WorkflowError::StageFailed(format!("probe {word} failed")))
    }
}

/// Re-enters by returning the outcome unchanged; pauses on call `pause_at`.
struct Reentry {
    calls: AtomicUsize,
    pause_at: usize,
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

fn setup() -> (tempfile::TempDir, WorkflowStore, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Accepted);
    (temp, store, run.id)
}

/// B1: a failure that keeps changing without clearing is re-entered at
/// most `REOPEN_LIMIT` times, then blocks the run by name.
#[tokio::test]
async fn reentry_stops_at_the_limit_and_blocks_by_name() {
    let (temp, store, run_id) = setup();
    let observer = EverNew {
        from: 0,
        calls: AtomicUsize::new(0),
    };
    let reentry = Reentry {
        calls: AtomicUsize::new(0),
        pause_at: 0,
    };
    let finalized = finalize(&store, &run_id, temp.path(), &observer, &reentry)
        .await
        .expect("finalizes");
    assert_eq!(finalized.status, WorkflowV2Status::NeedsReview);
    let next = finalized.next_action.unwrap();
    assert!(
        next.contains("limit") && next.contains("probe delta failed"),
        "{next}"
    );
    assert_eq!(reentry.calls.load(Ordering::SeqCst), REOPEN_LIMIT);
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        RunStatus::NeedsReview
    );
    let record = read_finalization(&store, &run_id);
    assert_eq!(record.prior_observer_failures.len(), REOPEN_LIMIT);
    assert!(!store.run_dir(&run_id).join(REOPEN_LEDGER_PATH).exists());
}

/// B1: re-entries made before a pause count after it; the resumed
/// finalization gets only what the limit leaves.
#[tokio::test]
async fn reentries_before_a_pause_count_toward_the_limit_after_it() {
    let (temp, store, run_id) = setup();
    let first = EverNew {
        from: 0,
        calls: AtomicUsize::new(0),
    };
    let paused = Reentry {
        calls: AtomicUsize::new(0),
        pause_at: 2,
    };
    let error = finalize(&store, &run_id, temp.path(), &first, &paused)
        .await
        .expect_err("the pause stops finalization");
    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    let ledger = ReopenLedger::load(&store, &run_id).unwrap();
    assert_eq!(ledger.reopens.len(), 2, "the paused re-entry counted");

    let resumed = EverNew {
        from: 6,
        calls: AtomicUsize::new(0),
    };
    let reentry = Reentry {
        calls: AtomicUsize::new(0),
        pause_at: 0,
    };
    let finalized = finalize(&store, &run_id, temp.path(), &resumed, &reentry)
        .await
        .expect("finalizes");
    assert_eq!(finalized.status, WorkflowV2Status::NeedsReview);
    assert_eq!(reentry.calls.load(Ordering::SeqCst), REOPEN_LIMIT - 2);
    let record = read_finalization(&store, &run_id);
    assert_eq!(record.prior_observer_failures.len(), REOPEN_LIMIT);
    assert!(!store.run_dir(&run_id).join(REOPEN_LEDGER_PATH).exists());
}
