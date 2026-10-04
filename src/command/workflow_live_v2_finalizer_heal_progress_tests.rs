//! Issue 262, round 2: the run-end re-entry guard counts re-entries since
//! the last real progress, never a total; and a stalled re-entry never
//! pauses a generation newer than the one that started finalizing.

use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::{
    ObserverAuthority, RunEndObserverOutcomeV1, RunStatus, WorkflowError, WorkflowResult,
    WorkflowStore, WorkflowV2Status,
};

use super::super::super::super::workflow_live_v2_script::WorkflowV2ScriptSummary;
use super::super::super::{RunEndObserverContext, WorkflowRunEndObserver};
use super::super::{Reopened, RunEndReopen};
use super::{Reentry, finalize_with, setup};

/// Each observation fails the next scripted set of checks, naming them,
/// then none: the run's failing set as each re-entry leaves it.
struct Shrinking {
    store: WorkflowStore,
    /// The failing check ids of each observation, in order; then none.
    sets: Vec<Vec<String>>,
    calls: AtomicUsize,
}

impl Shrinking {
    fn counting_down(store: WorkflowStore, counts: impl Iterator<Item = usize>) -> Self {
        let sets = counts
            .map(|n| (0..n).map(|id| format!("REQ-{id}")).collect())
            .collect();
        Self {
            store,
            sets,
            calls: AtomicUsize::new(0),
        }
    }
}

impl WorkflowRunEndObserver for Shrinking {
    fn observe(
        &self,
        context: &RunEndObserverContext<'_>,
    ) -> WorkflowResult<RunEndObserverOutcomeV1> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let ids = self.sets.get(call).cloned().unwrap_or_default();
        let failing = ids.len();
        let rows: String = ids
            .iter()
            .map(|id| format!("{{\"record_kind\":\"policy_shadow\",\"acceptance_id\":\"{id}\"}}\n"))
            .collect();
        let path =
            super::super::super::super::workflow_run_end_observer::RUN_END_OBSERVER_RECORDS_PATH;
        self.store
            .write_run_file(context.run_id, path, rows.as_bytes())
            .unwrap();
        Ok(RunEndObserverOutcomeV1 {
            authority: ObserverAuthority::ObserveOnly,
            evaluated_floor_count: 200,
            policy_finding_count: failing,
            operational_deferral_count: 0,
        })
    }
}

#[tokio::test]
async fn reentries_that_keep_shrinking_the_failure_never_pause_and_commit() {
    let (temp, store, run_id) = setup();
    let observer = Shrinking::counting_down(store.clone(), (1..=70).rev());
    let reentry = Reentry {
        calls: AtomicUsize::new(0),
        pause_at: 0,
    };
    let finalized = finalize_with(&store, &run_id, temp.path(), &observer, &reentry)
        .await
        .expect("progress never pauses");
    assert_eq!(finalized.status, WorkflowV2Status::Accepted);
    assert_eq!(reentry.calls.load(Ordering::SeqCst), 70);
}

/// Round 3 (decision A): after a regression from 1 failing check to 100,
/// every re-entry that repairs one more reaches a new failing set; none of
/// them pauses, although none beats the old minimum of one.
#[tokio::test]
async fn recovery_after_a_regression_never_pauses() {
    let (temp, store, run_id) = setup();
    let mut sets = vec![vec!["REQ-ONLY".to_string()]];
    sets.extend(
        (30..=100)
            .rev()
            .map(|n| (0..n).map(|id| format!("REQ-{id}")).collect()),
    );
    let count = sets.len();
    let observer = Shrinking {
        store: store.clone(),
        sets,
        calls: AtomicUsize::new(0),
    };
    let reentry = Reentry {
        calls: AtomicUsize::new(0),
        pause_at: 0,
    };
    finalize_with(&store, &run_id, temp.path(), &observer, &reentry)
        .await
        .expect("every re-entry reached a new failing set");
    assert_eq!(reentry.calls.load(Ordering::SeqCst), count);
}

/// A failing set reached before is a revisit, whatever its size.
#[tokio::test]
async fn a_revisited_failing_set_pauses() {
    let (temp, store, run_id) = setup();
    let set = |id: &str| vec![id.to_string()];
    let observer = Shrinking {
        store: store.clone(),
        sets: vec![set("REQ-1"), set("REQ-2"), set("REQ-1")],
        calls: AtomicUsize::new(0),
    };
    let reentry = Reentry {
        calls: AtomicUsize::new(0),
        pause_at: 0,
    };
    let error = finalize_with(&store, &run_id, temp.path(), &observer, &reentry)
        .await
        .expect_err("REQ-1 failing again is a revisit");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(reentry.calls.load(Ordering::SeqCst), 2);
}

/// The same failure every time.
struct Same;

impl WorkflowRunEndObserver for Same {
    fn observe(&self, _: &RunEndObserverContext<'_>) -> WorkflowResult<RunEndObserverOutcomeV1> {
        Err(WorkflowError::StageFailed(
            "probe failed the same way".into(),
        ))
    }
}

/// Re-enters, and meanwhile the operator pauses and resumes the run: a
/// newer generation owns it when the re-entry returns.
struct ResumedMeanwhile {
    store: WorkflowStore,
    run_id: String,
}

#[async_trait::async_trait]
impl RunEndReopen for ResumedMeanwhile {
    async fn reopen(&self, summary: &WorkflowV2ScriptSummary) -> WorkflowResult<Option<Reopened>> {
        let mut run = self.store.load_state(&self.run_id)?;
        run.generation += 2;
        self.store.save_state(&run)?;
        Ok(Some(Reopened {
            summary: summary.clone(),
            gate: None,
            record_path: None,
        }))
    }
}

#[tokio::test]
async fn a_stalled_reentry_never_pauses_a_newer_generation() {
    let (temp, store, run_id) = setup();
    let before = store.load_state(&run_id).unwrap();
    let reentry = ResumedMeanwhile {
        store: store.clone(),
        run_id: run_id.clone(),
    };
    let error = finalize_with(&store, &run_id, temp.path(), &Same, &reentry)
        .await
        .expect_err("the obsolete finalizer stops");
    assert!(
        matches!(error, WorkflowError::ControlCancelled(_)),
        "{error:?}"
    );
    let after = store.load_state(&run_id).unwrap();
    assert_eq!(after.generation, before.generation + 2);
    assert_ne!(after.status, RunStatus::Paused);
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap_or_default();
    assert!(
        !events.contains("run_end_acceptance_observer_stall_pause"),
        "{events}"
    );
}
