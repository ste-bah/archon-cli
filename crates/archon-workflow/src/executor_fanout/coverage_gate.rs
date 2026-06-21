use crate::acceptance;
use crate::error::{WorkflowError, WorkflowResult};
use crate::fanout::FanoutItem;
use crate::run::WorkflowRun;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;
use crate::work_unit_coverage::CoverageVerdict;
use crate::work_unit_gate;

pub(super) fn enforce_stage(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    items: &[FanoutItem],
    max_remediation_attempts: u32,
) -> WorkflowResult<()> {
    let payloads = items
        .iter()
        .map(|item| (item.id.clone(), item.payload.clone()))
        .collect::<Vec<_>>();
    let Some(coverage) =
        work_unit_gate::evaluate_agent_records(run, stage, payloads, &store.run_dir(&run.id))
    else {
        return Ok(());
    };
    let accepted = coverage.verdict == CoverageVerdict::Accepted;
    work_unit_gate::write_coverage_artifact(store, run, stage, &coverage, accepted)?;
    if accepted {
        return Ok(());
    }
    let sources = items
        .iter()
        .map(|item| crate::work_unit_remediation::source_from_payload(&item.payload))
        .collect();
    let remediation = crate::work_unit_remediation::write_missing_unit_items(
        store,
        run,
        stage,
        &coverage,
        sources,
        max_remediation_attempts,
    )?;
    let message = format!(
        "implementation fan-out stage '{}' rejected: {}",
        stage.id,
        work_unit_gate::error_summary(&coverage)
    );
    if remediation.attempts_exhausted {
        Err(WorkflowError::StageBlocked(message))
    } else {
        Err(WorkflowError::StageFailed(message))
    }
}

pub(super) fn evaluate_item(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    root: &std::path::Path,
    targets: &[String],
    after: &acceptance::TargetFingerprints,
) -> WorkflowResult<acceptance::AcceptanceOutcome> {
    if targets.is_empty() {
        return Ok(acceptance::AcceptanceOutcome::Rejected(
            "implementation stage declared no expected_target_files".to_string(),
        ));
    }
    let missing = acceptance::missing_targets(after);
    if !missing.is_empty() {
        return Ok(acceptance::AcceptanceOutcome::Rejected(format!(
            "expected_target_files missing after implementation: {}",
            missing.join(", ")
        )));
    }
    let Some(report) = crate::command_execution::run_verify_command(
        store,
        run,
        stage,
        root,
        stage.verify_command.as_deref(),
    )?
    else {
        return Ok(acceptance::AcceptanceOutcome::Accepted);
    };
    if report.success() {
        Ok(acceptance::AcceptanceOutcome::Accepted)
    } else {
        Ok(acceptance::AcceptanceOutcome::Rejected(
            report.failure_reason(),
        ))
    }
}
