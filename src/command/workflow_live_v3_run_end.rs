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
use archon_workflow::v2::acceptance_stage::{latest_round_record, relative_record_path};
use archon_workflow::{
    AuthoredAcceptanceGateV1, RunEndAcceptanceObserverSnapshotV1, WorkflowResult, WorkflowRunKind,
    WorkflowStore, WorkflowV2ResultStore, WorkflowV2Status,
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
    let run_dir = store.run_dir(run_id);
    let Some((record, path)) = latest_round_record(&run_dir)? else {
        return Ok((summary, None));
    };
    let record_path = relative_record_path(&run_dir, &path);
    let gate = AuthoredAcceptanceGateV1 {
        final_round: record.round,
        attempt: record.attempt,
        record_path: record_path.clone(),
        contract_present: record.contract_present,
        failing_check_ids: record.failing_check_ids(),
        unowned_failing_check_ids: record.unowned_failing_check_ids(),
        operational_errors: record.operational_errors.clone(),
    };
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

#[cfg(test)]
#[path = "workflow_live_v3_run_end_tests.rs"]
mod tests;
