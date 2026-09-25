//! Terminal rule of the authored (v3) lifecycle: acceptance decides (Obs-32).
//!
//! An authored run is `Complete` only when the final round its acceptance
//! stage recorded has zero failing checks. Anything else — a failing check, a
//! check no task implements, a stage that could not evaluate — ends the run
//! `NeedsReview`, naming the failing check ids in the summary and on
//! `v2/finalization.json`. This is the authored lifecycle's own rule, applied
//! here before the shared finalizer runs; the R2 run-end observer keeps its
//! observe-only contract untouched beside it.
//!
//! A run whose script never reached the stage (an older persisted script) has
//! no record and passes through unchanged — the pre-flight, not this gate,
//! decides which scripts must carry the stage.

use anyhow::Result;
use archon_workflow::v2::acceptance_stage::{
    AcceptanceRoundRecordV1, latest_round_record, relative_record_path,
};
use archon_workflow::v2::script::{
    AuthoredAcceptanceGateFact, AuthoredRunFacts, authored_run_terminal_status,
    is_acceptance_stage_call, review_remediation_verified_tasks,
};
use archon_workflow::{
    AuthoredAcceptanceGateV1, RunEndAcceptanceObserverSnapshotV1, WorkflowEventKind,
    WorkflowEventLog, WorkflowResult, WorkflowRunKind, WorkflowStore, WorkflowV2ResultStore,
    WorkflowV2Status,
};

use super::workflow_live_v2_script::WorkflowV2ScriptSummary;

/// Finalize a generated run. The authored kind passes through the acceptance
/// gate first; every other kind finalizes exactly as before. Returns the
/// summary the terminal record was made from, for the caller's report.
pub(super) async fn finalize_run(
    store: &WorkflowStore,
    run_id: &str,
    run_kind: WorkflowRunKind,
    observer_snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    summary: WorkflowV2ScriptSummary,
    v2_store: &WorkflowV2ResultStore,
) -> Result<WorkflowV2ScriptSummary> {
    let observer =
        super::workflow_run_end_observer::FixedRunEndAcceptanceObserver::new(store.clone());
    let (summary, gate) = if run_kind == WorkflowRunKind::AuthoredTaskWorkflow {
        apply_acceptance_gate(store, run_id, summary)?
    } else {
        (summary, None)
    };
    super::workflow_live_v2_finalizer::finalize_summary_with_gate(
        store,
        run_id,
        run_kind,
        observer_snapshot,
        &summary,
        v2_store,
        Some(&observer),
        None,
        gate,
    )
    .await?;
    Ok(summary)
}

/// Read the last acceptance round and hold the summary to it.
pub(super) fn apply_acceptance_gate(
    store: &WorkflowStore,
    run_id: &str,
    mut summary: WorkflowV2ScriptSummary,
) -> WorkflowResult<(WorkflowV2ScriptSummary, Option<AuthoredAcceptanceGateV1>)> {
    let Some((gate, record, path)) = read_acceptance_gate(store, run_id)? else {
        return Ok((summary, None));
    };
    let record_path = gate.record_path.clone();
    if !gate.blocks_completion() {
        return Ok((summary, Some(gate)));
    }
    let owners = record
        .failing_checks()
        .iter()
        .map(|check| {
            if check.owning_tasks.is_empty() {
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
    let reason = if gate.failing_check_ids.is_empty() {
        format!(
            "the acceptance stage could not evaluate the frozen checks in round {}: {}",
            record.round,
            record.operational_errors.join("; ")
        )
    } else {
        format!(
            "frozen acceptance checks still fail after round {}: {owners}",
            record.round
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
            "{reason}; fix the named tasks' implementation against the contract and /workflow resume --live {run_id}, or inspect {record_path}"
        ));
    }
    Ok((summary, Some(gate)))
}

/// The last acceptance round the host recorded, as the gate the finalization
/// record carries.
fn read_acceptance_gate(
    store: &WorkflowStore,
    run_id: &str,
) -> WorkflowResult<
    Option<(
        AuthoredAcceptanceGateV1,
        AcceptanceRoundRecordV1,
        std::path::PathBuf,
    )>,
> {
    let run_dir = store.run_dir(run_id);
    let Some((record, path)) = latest_round_record(&run_dir)? else {
        return Ok(None);
    };
    let gate = AuthoredAcceptanceGateV1 {
        final_round: record.round,
        attempt: record.attempt,
        record_path: relative_record_path(&run_dir, &path),
        contract_present: record.contract_present,
        failing_check_ids: record.failing_check_ids(),
        unowned_failing_check_ids: record.unowned_failing_check_ids(),
        operational_errors: record.operational_errors.clone(),
    };
    Ok(Some((gate, record, path)))
}

/// Replace the accumulator's worst-call status with the verdict of the run's
/// final accounting (`authored_run_terminal_status`), and record why in
/// `events.jsonl`. Hard stops keep their status; see the rule's module doc.
///
/// Runs after the executed-run validators, so the accounting it reads has
/// already been checked for shape, task partition and review findings.
pub(super) fn apply_authored_run_outcome(
    store: &WorkflowStore,
    run_id: &str,
    v2_store: &WorkflowV2ResultStore,
    acceptance_required: bool,
    mut summary: WorkflowV2ScriptSummary,
) -> WorkflowResult<WorkflowV2ScriptSummary> {
    let accumulated = summary.status;
    // The gate is judged against the last acceptance call THIS run executed
    // or replayed, and the host's own record of it -- never the script's copy.
    let gate = read_acceptance_gate(store, run_id)?;
    let last_call = summary
        .calls
        .iter()
        .rev()
        .find(|call| is_acceptance_stage_call(call));
    let last_call_status = match last_call {
        Some(call) => v2_store
            .load_call_record(&call.id)?
            .filter(|record| record.invalidated_by.is_none())
            .map(|record| record.status),
        None => None,
    };
    let gate_fact = match (&gate, last_call) {
        (Some((gate, record, _)), Some(call)) => AuthoredAcceptanceGateFact::Recorded {
            gate,
            record_call_id: &record.call_id,
            last_call_id: &call.id,
            last_call_status,
        },
        (None, None) if !acceptance_required => AuthoredAcceptanceGateFact::NotRequired,
        _ => AuthoredAcceptanceGateFact::Missing,
    };
    let verified = review_remediation_verified_tasks(&summary.calls, |call_id| {
        Ok(v2_store
            .load_call_record(call_id)?
            .filter(|record| record.invalidated_by.is_none())
            .map(|record| record.status))
    })?;
    let outcome = authored_run_terminal_status(&AuthoredRunFacts {
        accumulated_status: summary.status,
        host_terminal_failure: summary.failed_call.as_deref(),
        script_result: summary.script_result.as_deref(),
        acceptance_gate: gate_fact,
        verified_remediation_tasks: &verified,
    });
    let explanation = outcome.explanation();
    if outcome.from_accounting {
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
#[path = "workflow_live_v3_run_end_tests.rs"]
mod tests;
