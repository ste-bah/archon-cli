//! Terminal rule of the authored (v3) lifecycle: acceptance decides (Obs-32).
//!
//! An authored run is `Complete` only when the final round its acceptance
//! stage recorded has zero failing checks. Anything else — a failing check, a
//! check no task implements, a stage that could not evaluate — ends the run
//! `NeedsReview`, naming the failing check ids in the summary and on
//! `v2/finalization.json`. This is the authored lifecycle's own rule, applied
//! here before the shared finalizer runs. The run-end observer then runs
//! before the terminal commit (ACC-A9): a failed observation re-enters this
//! run's acceptance stage in the same session (`reopen`) and the outcome is
//! held to the round it records.
//!
//! REM-13: a script that returns without the stage (one authored before the
//! rule) has the prelude run it after its last call, so every authored run
//! records a round. One that still recorded none never ran its checks, and
//! the terminal rule blocks it (`AuthoredAcceptanceGateFact::NotRequired`
//! is no longer a pass).

use anyhow::Result;
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::acceptance_stage::AcceptanceRoundRecordV1;
use archon_workflow::v2::script::residual_plan::residual_verdict;
use archon_workflow::v2::script::{
    AuthoredAcceptanceGateFact, AuthoredCallRole, AuthoredRunFacts, authored_call_facts,
    authored_run_terminal_status_with, is_acceptance_stage_call, writable_task_ids,
};
use archon_workflow::{
    AuthoredAcceptanceGateV1, RunEndAcceptanceObserverSnapshotV1, WorkflowEventKind,
    WorkflowEventLog, WorkflowResult, WorkflowRunKind, WorkflowStore, WorkflowV2ResultStore,
    WorkflowV2Status,
};

use super::workflow_live_v2_script::WorkflowV2ScriptSummary;

#[path = "workflow_live_v3_run_end_call.rs"]
mod call;
#[path = "workflow_live_v3_run_end_gate.rs"]
mod gate;
use gate::{GateRecord, gate_of, read_acceptance_gate};
#[path = "workflow_live_v3_owned_pause.rs"]
pub(super) mod owned_pause;
#[path = "workflow_live_v3_run_end_reopen.rs"]
mod reopen;
#[path = "workflow_live_v3_run_end_stop.rs"]
mod stop_control;
pub(super) use reopen::AcceptanceReentry;
pub(super) use stop_control::stop;

/// Finalize a generated run. The authored kind passes through the acceptance
/// gate first; every other kind finalizes exactly as before. Returns the
/// summary the terminal record was made from, for the caller's report.
#[cfg(test)]
pub(super) async fn finalize_run(
    store: &WorkflowStore,
    run_id: &str,
    run_kind: WorkflowRunKind,
    observer_snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    summary: WorkflowV2ScriptSummary,
    v2_store: &WorkflowV2ResultStore,
) -> Result<WorkflowV2ScriptSummary> {
    finalize_run_reentering(
        store,
        run_id,
        run_kind,
        observer_snapshot,
        summary,
        v2_store,
        None,
        None,
    )
    .await
}

/// [`finalize_run`], with what the run's acceptance stage runs on so a failed
/// pre-commit observation can re-enter it (ACC-A9). With `None` a failed
/// observation of a finishing outcome blocks the run by name.
pub(super) async fn finalize_run_reentering(
    store: &WorkflowStore,
    run_id: &str,
    run_kind: WorkflowRunKind,
    observer_snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    summary: WorkflowV2ScriptSummary,
    v2_store: &WorkflowV2ResultStore,
    reentry: Option<AcceptanceReentry<'_>>,
    expected_generation: Option<u64>,
) -> Result<WorkflowV2ScriptSummary> {
    let observer =
        super::workflow_run_end_observer::FixedRunEndAcceptanceObserver::new(store.clone());
    finalize_run_observed(
        store,
        run_id,
        run_kind,
        observer_snapshot,
        summary,
        v2_store,
        &observer,
        reentry,
        expected_generation,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn finalize_run_observed(
    store: &WorkflowStore,
    run_id: &str,
    run_kind: WorkflowRunKind,
    observer_snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    summary: WorkflowV2ScriptSummary,
    v2_store: &WorkflowV2ResultStore,
    observer: &dyn super::workflow_live_v2_finalizer::WorkflowRunEndObserver,
    reentry: Option<AcceptanceReentry<'_>>,
    expected_generation: Option<u64>,
) -> Result<WorkflowV2ScriptSummary> {
    let (summary, gate) = if run_kind == WorkflowRunKind::AuthoredTaskWorkflow {
        apply_acceptance_gate(store, run_id, v2_store, summary)?
    } else {
        (summary, None)
    };
    let reopen = reentry
        .filter(|_| run_kind == WorkflowRunKind::AuthoredTaskWorkflow)
        .map(|reentry| {
            reopen::AcceptanceReopen::new(store, run_id, reentry, (expected_generation, v2_store))
        });
    let finalized = super::workflow_live_v2_finalizer::finalize_summary_with_gate(
        store,
        run_id,
        run_kind,
        observer_snapshot,
        &summary,
        v2_store,
        Some(observer),
        expected_generation,
        gate,
        reopen
            .as_ref()
            .map(|reopen| reopen as &dyn super::workflow_live_v2_finalizer::RunEndReopen),
    )
    .await;
    // Issue 316: a refused pause is never written as a cancellation.
    let finalized = match finalized {
        Err(archon_workflow::WorkflowError::ControlCancelled(message)) => Err(
            stop_control::refused_while_owned(store, run_id, expected_generation, &message)
                .unwrap_or(archon_workflow::WorkflowError::ControlCancelled(message)),
        ),
        other => other,
    };
    match finalized {
        Ok(summary) => Ok(summary),
        // Re-entered acceptance honours run control: the run stops resumable.
        Err(archon_workflow::WorkflowError::ControlPaused(message)) => {
            stop(
                store,
                run_id,
                run_kind,
                archon_workflow::RunStatus::Paused,
                &message,
                expected_generation,
            )?;
            Err(archon_workflow::WorkflowError::ControlPaused(message).into())
        }
        Err(archon_workflow::WorkflowError::ControlCancelled(message)) => {
            stop(
                store,
                run_id,
                run_kind,
                archon_workflow::RunStatus::Cancelled,
                &message,
                expected_generation,
            )?;
            Err(archon_workflow::WorkflowError::ControlCancelled(message).into())
        }
        Err(error) => Err(error.into()),
    }
}

/// Read the last acceptance round and hold the summary to it.
pub(super) fn apply_acceptance_gate(
    store: &WorkflowStore,
    run_id: &str,
    v2_store: &WorkflowV2ResultStore,
    summary: WorkflowV2ScriptSummary,
) -> WorkflowResult<(WorkflowV2ScriptSummary, Option<AuthoredAcceptanceGateV1>)> {
    let Some(GateRecord {
        gate, record, path, ..
    }) = read_acceptance_gate(store, run_id, v2_store, &summary.calls)?
    else {
        return Ok((summary, None));
    };
    Ok(hold_to_round(run_id, summary, gate, &record, &path))
}

/// The gate `record` (read from `path`) gives, and `summary` held to it: a
/// blocking round turns a finishing or reviewable outcome into `NeedsReview`
/// naming the failing checks' owners.
fn hold_to_round(
    run_id: &str,
    mut summary: WorkflowV2ScriptSummary,
    gate: AuthoredAcceptanceGateV1,
    record: &AcceptanceRoundRecordV1,
    path: &std::path::Path,
) -> (WorkflowV2ScriptSummary, Option<AuthoredAcceptanceGateV1>) {
    let record_path = gate.record_path.clone();
    if !gate.blocks_completion() {
        return (summary, Some(gate));
    }
    let owners = record
        .failing_checks()
        .iter()
        .map(|check| {
            if check.contract_defect {
                format!(
                    "{} (contract defect: the frozen check was not accepted by the judge; re-author it)",
                    check.check_id
                )
            } else if check.owning_tasks.is_empty() {
                format!("{} (no task implements it)", check.check_id)
            } else {
                format!(
                    "{} (owned by {})",
                    check.check_id,
                    check.owning_tasks.join(", ")
                )
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    // An operational stop names its own remedy (missing recovery evidence,
    // an unreadable store); only failing checks point at the tasks' code.
    let (reason, remedy) = if gate.failing_check_ids.is_empty() {
        (
            format!(
                "the acceptance stage could not evaluate the frozen checks in round {}: {}",
                record.round,
                record.operational_errors.join("; ")
            ),
            "resolve the operational errors above as each one says",
        )
    } else {
        (
            format!(
                "frozen acceptance checks still fail after round {}: {owners}",
                record.round
            ),
            "fix the named tasks' implementation against the contract",
        )
    };
    if matches!(
        summary.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop | WorkflowV2Status::NeedsReview
    ) {
        summary.status = WorkflowV2Status::NeedsReview;
        summary.failed_call = Some(record.call_id.clone());
        summary.failed_result_path = Some(path.display().to_string());
        summary.next_action = Some(format!(
            "{reason}; {remedy} and /workflow resume --live {run_id}, or inspect {record_path}"
        ));
    }
    (summary, Some(gate))
}

/// Replace the accumulator's worst-call status with the verdict the host's
/// own records give (`authored_run_terminal_status`), and record why in
/// `events.jsonl`. Hard stops keep their status; see the rule's module doc.
///
/// Runs after the executed-run validators, so the accounting it reads has
/// already been checked for shape, task partition and review findings; every
/// list in it is still only a claim the rule checks against the records.
#[cfg(test)]
pub(super) fn apply_authored_run_outcome(
    store: &WorkflowStore,
    run_id: &str,
    v2_store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&std::path::Path>,
    acceptance_required: bool,
    summary: WorkflowV2ScriptSummary,
) -> WorkflowResult<WorkflowV2ScriptSummary> {
    apply_authored_run_outcome_with(
        store,
        run_id,
        v2_store,
        universe,
        repository_root,
        acceptance_required,
        summary,
        archon_workflow::v2::verification::regression_gate::RegressionVerdict::default(),
    )
}

/// Pauses `run_id` for `owner` because the final gate found residual gaps
/// that stand only for want of progress. The owner is checked with the pause
/// under the run lock (Issue 316): a stale executor never pauses the new
/// owner's run.
fn pause_on_residual_stall(
    store: &WorkflowStore,
    run_id: &str,
    owner: archon_workflow::control_pause::PauseOwner,
    stalled: &[String],
) -> archon_workflow::WorkflowError {
    let detail = serde_json::json!({
        "event": "residual_gate_stall_pause", "cause": "no_progress", "stalled": stalled,
    });
    match owned_pause::pause(store, run_id, owner, detail) {
        Ok(event) => {
            if let Err(error) = event {
                tracing::warn!(%error, "residual gate pause event not recorded");
            }
            archon_workflow::WorkflowError::ControlPaused(format!(
                "the residual passes made no progress on {} gap(s); run {run_id} is paused, not failed: fix what they name, then workflow resume {run_id}",
                stalled.len()
            ))
        }
        Err(error) => error,
    }
}

/// [`apply_authored_run_outcome`], with the final gate's regression check
/// (Issue-114, `regression_gate`) folded in: a new failure blocks, a
/// pre-existing one is listed.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_authored_run_outcome_with(
    store: &WorkflowStore,
    run_id: &str,
    v2_store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&std::path::Path>,
    acceptance_required: bool,
    mut summary: WorkflowV2ScriptSummary,
    regression: archon_workflow::v2::verification::regression_gate::RegressionVerdict,
) -> WorkflowResult<WorkflowV2ScriptSummary> {
    let accumulated = summary.status;
    // Issue 313: an acceptance call's damaged record pauses the run; it is
    // never read as a stage that never ran, nor guessed from a round record.
    let facts = authored_call_facts(&summary.calls, |call_id| {
        match (summary.calls.iter())
            .find(|call| call.id == call_id && is_acceptance_stage_call(call))
        {
            Some(call) => call::acceptance_call_record(store, run_id, v2_store, call),
            None => v2_store.load_call_record(call_id),
        }
    })?;
    let gate = read_acceptance_gate(store, run_id, v2_store, &summary.calls)?;
    let last_acceptance = facts
        .iter()
        .rev()
        .find(|fact| fact.role == AuthoredCallRole::Acceptance);
    let gate_fact = match (&gate, last_acceptance) {
        (Some(gate), Some(call)) if gate.bound => AuthoredAcceptanceGateFact::Recorded {
            gate: &gate.gate,
            record_call_id: &gate.record.call_id,
            last_call_id: &call.id,
            last_call_status: call.status,
        },
        (None, None) if !acceptance_required => AuthoredAcceptanceGateFact::NotRequired,
        _ => AuthoredAcceptanceGateFact::Missing,
    };
    // An authored run always has a universe: `run_generated_v2_workflow`
    // refuses a v3 run without one (workflow_live_v2_run.rs:378) and only
    // enters the authored lifecycle when one is present (:398). Only a direct
    // unit-test call reaches here without it, and then every task id is
    // unknown, which holds the run — the fail-safe direction.
    let writable = writable_task_ids(universe);
    let universe_tasks = universe
        .map(|universe| {
            universe
                .tasks
                .iter()
                .map(|task| task.canonical_task_id.clone())
                .collect()
        })
        .unwrap_or_default();
    // Issue-117: the residual gaps accepted verifiers recorded, and the
    // review units a host-planned round completed.
    let residual = residual_verdict(&summary.calls, v2_store, universe, repository_root);
    // A stall is never terminal: gaps that stand only because the residual
    // passes stopped making progress pause the run, with the evidence, and
    // the resume plans the stalled pass again (rule A).
    if !residual.stalled.is_empty() && summary.failed_call.is_none() {
        let owner = call::session_owner(v2_store);
        return Err(pause_on_residual_stall(
            store,
            run_id,
            owner,
            &residual.stalled,
        ));
    }
    let outcome = authored_run_terminal_status_with(
        &AuthoredRunFacts {
            accumulated_status: summary.status,
            host_terminal_failure: summary.failed_call.as_deref(),
            script_result: summary.script_result.as_deref(),
            acceptance_gate: gate_fact,
            calls: &facts,
            writable_tasks: &writable,
            universe_tasks: &universe_tasks,
        },
        &residual.discharged,
    )
    .with_residual_gate(residual.blocking, residual.notes)
    .with_residual_gate(regression.blocking, regression.notes);
    let explanation = outcome.explanation();
    // Issue 293: a hard stop keeps its own status and evidence, unless the
    // rule found that status a pass it cannot be (a stopped run is incomplete).
    if outcome.from_accounting || outcome.status != summary.status {
        summary.status = outcome.status;
        summary.next_action = (!matches!(
            outcome.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ))
        .then(|| {
            format!("{explanation}; address the named items and /workflow resume --live {run_id}")
        });
    }
    let detail = serde_json::json!({
        "event": "authored_run_outcome",
        "status": outcome.status,
        "accumulated_status": accumulated,
        "from_accounting": outcome.from_accounting,
        "blocking": outcome.blocking,
        "notes": outcome.notes,
        "explanation": explanation,
    });
    let kind = match outcome.status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => WorkflowEventKind::StageCompleted,
        WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => WorkflowEventKind::StageFailed,
        _ => WorkflowEventKind::StageStalled,
    };
    // Best-effort: a log write must never change the run's outcome.
    if let Err(error) = store
        .next_event_seq(run_id)
        .and_then(|seq| WorkflowEventLog::new(store.clone()).emit(run_id, seq, kind, detail))
    {
        tracing::warn!(%error, run_id, "could not record the authored run outcome event");
    }
    Ok(summary)
}

#[cfg(test)]
#[path = "workflow_live_v3_run_end_heal_tests.rs"]
mod heal_tests;
#[cfg(test)]
#[path = "workflow_live_v3_run_end_tests.rs"]
mod tests;
