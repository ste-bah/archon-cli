//! ACC-A9: the run-end observation runs BEFORE the terminal commit and can
//! change the outcome.
//!
//! A pending observation is made on the uncommitted outcome. When it cannot
//! complete, or completes with failing checks (`policy_finding_count > 0`),
//! while the outcome would finish the run, no passing status is committed:
//! the failure is kept on the record and acceptance is re-entered
//! (`RunEndReopen`: the authored lifecycle re-runs its acceptance stage in the
//! same session, on the chain the pin names now, which the stage and the
//! observer both hold to the launch pin through the recorded lineage path),
//! so a failing check is routed to its owners by the round. The re-entered
//! round re-decides the outcome and the observation runs again. Re-entry
//! repeats while it makes progress. Progress is reaching a failing set (the sorted
//! failing check ids, never the failure text) the run never reached
//! (decision A); `REOPEN_STALL_LIMIT` refusals in a row that reach one
//! already reached are a stall, and a stall PAUSES the run with its
//! evidence (Issue 262): never a total count that ends the run. A failure no
//! re-entry can act on (no acceptance stage, or a re-entry that errs) blocks
//! the run by name: `NeedsReview`, naming the observation's reason or failing
//! checks.
//!
//! An outcome that does not finish the run is not re-entered: nothing passing
//! can be committed on it, so the failure is recorded beside it.

use archon_workflow::{
    AuthoredAcceptanceGateV1, FinalizationRecordV1, RunEndObserverOutcomeV1, WorkflowError,
    WorkflowEventKind, WorkflowResult, WorkflowStore, WorkflowV2Status,
};

use super::super::workflow_live_v2_script::WorkflowV2ScriptSummary;
use super::{
    FINALIZATION_RECORD_PATH, RunEndObserverContext, WorkflowRunEndObserver, emit_observer_event,
    require_generation_owner,
};

/// What a re-entered acceptance stage recorded.
pub(in super::super) struct Reopened {
    pub(in super::super) summary: WorkflowV2ScriptSummary,
    pub(in super::super) gate: Option<AuthoredAcceptanceGateV1>,
    /// The re-entered round's record, relative to the run directory.
    pub(in super::super) record_path: Option<String>,
}

/// Re-entry of the acceptance stage a failed run-end observation reopens.
#[async_trait::async_trait]
pub(in super::super) trait RunEndReopen: Send + Sync {
    /// Re-run acceptance for `summary`'s run in this session and return the
    /// outcome its round decides, or `None` when the run has no acceptance
    /// stage to re-enter.
    async fn reopen(&self, summary: &WorkflowV2ScriptSummary) -> WorkflowResult<Option<Reopened>>;
}

pub(super) struct PreCommit<'a> {
    pub(super) store: &'a WorkflowStore,
    pub(super) run_id: &'a str,
    pub(super) expected_generation: Option<u64>,
    pub(super) observer: &'a dyn WorkflowRunEndObserver,
    pub(super) reopen: Option<&'a dyn RunEndReopen>,
}

/// The call id a failure the observation still names is blocked under.
pub(super) const OBSERVER_BLOCK_CALL_ID: &str = "run-end-acceptance-observer";

fn finishes(status: WorkflowV2Status) -> bool {
    matches!(status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop)
}

/// Refusals in a row that reach a failing set the run already reached,
/// after which the run pauses: the no-progress bound (decision A). One: a
/// re-entry that leaves the run where an earlier one did gives the next
/// nothing new to act on. A new failing set resets it, whatever its size;
/// the sets reached are kept in the ledger across pauses.
pub(super) const REOPEN_STALL_LIMIT: usize = 1;

/// Why an observation does not let a finishing outcome commit.
struct Refusal {
    reason: String,
    /// The completed observation, when it completed with failing checks.
    outcome: Option<RunEndObserverOutcomeV1>,
}

/// Observe the uncommitted outcome, re-entering acceptance while that makes
/// progress; returns the outcome to commit. `record` leaves with its
/// observation completed or failed.
pub(super) async fn observe_before_commit(
    pc: &PreCommit<'_>,
    mut summary: WorkflowV2ScriptSummary,
    record: &mut FinalizationRecordV1,
) -> WorkflowResult<WorkflowV2ScriptSummary> {
    emit_observer_event(
        pc.store,
        pc.run_id,
        WorkflowEventKind::RunEndAcceptanceObserverStarted,
        "run_end_acceptance_observer_started",
        serde_json::json!({"authority": "observe_only", "before_terminal_commit": true}),
    )?;
    // Only the generation that started this finalization may pause the run.
    let generation = match pc.expected_generation {
        Some(generation) => generation,
        None => pc.store.load_state(pc.run_id)?.generation,
    };
    // Re-entries a paused or interrupted finalization already made count.
    let mut ledger = state::ReopenLedger::load(pc.store, pc.run_id)?;
    if !ledger.reopens.is_empty() {
        record.prior_observer_failures = ledger.reopens.clone();
    }
    loop {
        let snapshot = record.observer_snapshot.clone().ok_or_else(|| {
            WorkflowError::StateCorrupt(
                "observer_pending finalization has no launch-time observer snapshot".to_string(),
            )
        })?;
        let context = RunEndObserverContext {
            run_id: pc.run_id,
            terminal_status: summary.status,
            snapshot: &snapshot,
            pre_commit: true,
        };
        let refusal = match pc.observer.observe_async(&context).await {
            // Findings never commit a finishing outcome (B2): the failing
            // checks re-open acceptance like any other failure.
            Ok(outcome) if outcome.policy_finding_count > 0 && finishes(summary.status) => {
                let checks = state::shadowed_checks(pc.store, pc.run_id);
                Refusal {
                    reason: format!(
                        "the run-end observation found {} failing frozen check result(s): {}",
                        outcome.policy_finding_count,
                        if checks.is_empty() {
                            "see the observer records".to_string()
                        } else {
                            checks.join(", ")
                        }
                    ),
                    outcome: Some(outcome),
                }
            }
            Ok(outcome) => {
                record.complete_observer(outcome)?;
                return Ok(summary);
            }
            Err(error) => Refusal {
                reason: error.to_string(),
                outcome: None,
            },
        };
        let reason = refusal.reason.clone();
        if !finishes(summary.status) {
            fail(pc, record, &reason)?;
            return Ok(summary);
        }
        let situation = state::situation(pc.store, &snapshot, &reason, &summary, record);
        // Decision A: progress is a failing set the run never reached.
        if ledger
            .seen
            .insert(state::failing_set(pc.store, pc.run_id, record))
        {
            ledger.revisits = 0;
        } else {
            ledger.revisits = ledger.revisits.saturating_add(1);
        }
        if ledger.revisits >= REOPEN_STALL_LIMIT {
            return Err(pause_reentry(
                pc,
                generation,
                record,
                (&reason, &situation),
                "no_progress",
                "re-entering acceptance made no progress: the run reached a failing set it had already reached",
            ));
        }
        let why = if let Some(reopen) = pc.reopen {
            state::keep_native_evidence(pc.store, pc.run_id, record.prior_observer_failures.len())?;
            // Counted before the stage runs, so a pause inside it keeps it.
            record.reopen_before_commit(reason.clone())?;
            ledger.reopens = record.prior_observer_failures.clone();
            ledger.save(pc.store, pc.run_id, pc.expected_generation)?;
            match reopen.reopen(&summary).await {
                Ok(Some(reopened)) => {
                    record.restate(reopened.summary.status, reopened.gate.clone())?;
                    emit(
                        pc,
                        WorkflowEventKind::RunEndAcceptanceObserverStarted,
                        serde_json::json!({
                            "event": "run_end_acceptance_reopened",
                            "reason": reason,
                            "reopen": record.prior_observer_failures.len(),
                            "status": reopened.summary.status,
                            "record_path": reopened.record_path,
                        }),
                    )?;
                    summary = reopened.summary;
                    continue;
                }
                Ok(None) => "the run records no acceptance stage to re-enter".to_string(),
                Err(error @ WorkflowError::ControlPaused(_))
                | Err(error @ WorkflowError::ControlCancelled(_)) => return Err(error),
                Err(error) => format!("re-entering acceptance failed: {error}"),
            }
        } else {
            "this finalization has no acceptance stage to re-enter".to_string()
        };
        block_by_name(pc, &mut summary, record, refusal, &why)?;
        return Ok(summary);
    }
}

/// Re-entry stopped making progress (Issue 262): the run is paused with the
/// evidence, never ended; returns the control error finalization ends with.
/// The ledger keeps every re-entry, so a resume continues the count.
fn pause_reentry(
    pc: &PreCommit<'_>,
    generation: u64,
    record: &FinalizationRecordV1,
    (reason, situation): (&str, &str),
    cause: &'static str,
    why: &str,
) -> WorkflowError {
    let message = format!(
        "the run-end acceptance observation failed before the terminal commit and {why}: {reason}; the run is paused, not failed: fix what it names, then archon workflow resume --live --yes {}",
        pc.run_id
    );
    let detail = serde_json::json!({
        "event": "run_end_acceptance_observer_stall_pause",
        "cause": cause,
        "why": why,
        "reason": reason,
        "observation": situation,
        "reopens": record.prior_observer_failures.len(),
        "prior_observer_failures": record.prior_observer_failures,
    });
    // Owned by the generation that started this finalization: an operator
    // pause and resume meanwhile makes this finalizer obsolete, and it stops.
    match archon_workflow::control_pause::pause_with_evidence(
        pc.store, pc.run_id, generation, detail,
    ) {
        Ok(event) => {
            if let Err(error) = event {
                tracing::warn!(%error, "run-end observer pause event not recorded");
            }
            tracing::warn!(run_id = pc.run_id, "{message}");
            WorkflowError::ControlPaused(message)
        }
        Err(error) => error,
    }
}

/// The failure still stands: the run ends `NeedsReview`, naming it.
fn block_by_name(
    pc: &PreCommit<'_>,
    summary: &mut WorkflowV2ScriptSummary,
    record: &mut FinalizationRecordV1,
    refusal: Refusal,
    why: &str,
) -> WorkflowResult<()> {
    let reason = refusal.reason.as_str();
    record.restate(
        WorkflowV2Status::NeedsReview,
        record.acceptance_gate.clone(),
    )?;
    summary.status = WorkflowV2Status::NeedsReview;
    summary.failed_call = Some(OBSERVER_BLOCK_CALL_ID.to_string());
    summary.failed_result_path = Some(
        pc.store
            .run_dir(pc.run_id)
            .join(FINALIZATION_RECORD_PATH)
            .display()
            .to_string(),
    );
    summary.next_action = Some(format!(
        "the run-end acceptance observation failed before the terminal commit and {why}: {reason}; fix what it names and /workflow resume --live {}",
        pc.run_id
    ));
    emit(
        pc,
        WorkflowEventKind::RunEndAcceptanceObserverFailed,
        serde_json::json!({
            "event": "run_end_acceptance_observer_blocked",
            "reason": reason,
            "why": why,
            "reopens": record.prior_observer_failures.len(),
        }),
    )?;
    match refusal.outcome {
        // The observation completed; its failing checks are what blocks.
        Some(outcome) => record.complete_observer(outcome),
        None => fail(pc, record, reason),
    }
}

fn fail(pc: &PreCommit<'_>, record: &mut FinalizationRecordV1, reason: &str) -> WorkflowResult<()> {
    record.fail_observer(reason.to_string())?;
    pc.store.with_run_lock(pc.run_id, |locked| {
        require_generation_owner(locked, pc.run_id, pc.expected_generation)?;
        emit_observer_event(
            locked,
            pc.run_id,
            WorkflowEventKind::RunEndAcceptanceObserverFailed,
            "run_end_acceptance_observer_failed",
            serde_json::json!({"reason": reason, "before_terminal_commit": true}),
        )
    })
}

fn emit(
    pc: &PreCommit<'_>,
    kind: WorkflowEventKind,
    detail: serde_json::Value,
) -> WorkflowResult<()> {
    pc.store.with_run_lock(pc.run_id, |locked| {
        require_generation_owner(locked, pc.run_id, pc.expected_generation)?;
        let seq = locked.next_event_seq(pc.run_id)?;
        archon_workflow::WorkflowEventLog::new(locked.clone())
            .emit(pc.run_id, seq, kind, detail)
            .map(|_| ())
    })
}

#[path = "workflow_live_v2_finalizer_heal_state.rs"]
mod state;
pub(super) use state::clear_reopen_ledger;

#[cfg(test)]
#[path = "workflow_live_v2_finalizer_heal_tests.rs"]
mod tests;
