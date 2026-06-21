use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::json;

use crate::error::{WorkflowError, WorkflowResult};
use crate::fanout;
use crate::persistence;
use crate::run::{StageStatus, WorkflowRun};
use crate::spec::StageSpec;
use crate::store::WorkflowStore;
use crate::work_unit_coverage::{self, CoverageVerdict, EvidenceBundle, WorkUnitCoverage};
use crate::work_unit_gate;
use crate::write_coordinator::{CoordinatedOutcome, ManifestStatus};

use super::coordinated_failure::coordinated_item_body;
use super::coordinated_success::{coordinated_accepted_item_body, coordinated_target_files};

pub(super) fn record_coordinated_outcome(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    outcome: &CoordinatedOutcome,
    max_remediation_attempts: u32,
) -> WorkflowResult<()> {
    let run_dir = store.run_dir(&run.id);
    let coverage = stage_coverage(run, stage, outcome, &run_dir);
    let coverage_required = !coverage.required_work_units.is_empty();
    let coverage_accepted = coverage.verdict == CoverageVerdict::Accepted;
    let coverage_artifact_key = work_unit_gate::artifact_key(run, stage, "__work_unit_coverage");
    let mut remediation_exhausted = false;
    if coverage_required {
        work_unit_gate::write_coverage_artifact(store, run, stage, &coverage, coverage_accepted)?;
        if !coverage_accepted {
            let sources = outcome
                .plans
                .iter()
                .map(|plan| {
                    crate::work_unit_remediation::source_from_plan(
                        plan.work_unit_ids.clone(),
                        plan.target_files.clone(),
                    )
                })
                .collect();
            remediation_exhausted = crate::work_unit_remediation::write_missing_unit_items(
                store,
                run,
                stage,
                &coverage,
                sources,
                max_remediation_attempts,
            )?
            .attempts_exhausted;
        }
    }

    let bundles_by_item = bundles_by_item(outcome, &run_dir, stage);
    let mut failures = Vec::new();
    for (item_id, status) in &outcome.item_status {
        let manifest_accepted = manifest_accepts(status);
        let item_coverage = item_coverage(run, stage, outcome, item_id, &bundles_by_item);
        let item_coverage_accepted = item_coverage
            .as_ref()
            .is_none_or(|coverage| coverage.verdict == CoverageVerdict::Accepted);
        let accepted = manifest_accepted
            && (!coverage_required || (coverage_accepted && item_coverage_accepted));
        let error = item_error(status, accepted, item_coverage.as_ref(), &coverage);
        let body_coverage = failing_coverage(item_coverage.as_ref(), &coverage);
        if let Some(err) = &error {
            failures.push(format!("{item_id}: {err}"));
        }
        let body = item_body(
            &run_dir,
            stage,
            outcome,
            item_id,
            status,
            accepted,
            error.as_deref(),
            body_coverage,
            &coverage_artifact_key,
        );
        let artifact = persistence::write_attached_stage_artifact(
            store, run, stage, item_id, "md", body, accepted,
        )?;
        fanout::record_item(
            run,
            stage,
            item_id.clone(),
            if accepted {
                StageStatus::Accepted
            } else {
                StageStatus::Failed
            },
            Some(artifact),
            error,
        );
    }
    if failures.is_empty() {
        Ok(())
    } else if remediation_exhausted {
        Err(WorkflowError::StageBlocked(format!(
            "coordinated implementation fan-out blocked after remediation cap: {}",
            failures.join("; ")
        )))
    } else {
        Err(WorkflowError::StageFailed(format!(
            "coordinated implementation fan-out failed: {}",
            failures.join("; ")
        )))
    }
}

fn stage_coverage(
    run: &WorkflowRun,
    stage: &StageSpec,
    outcome: &CoordinatedOutcome,
    run_dir: &Path,
) -> WorkUnitCoverage {
    let mut required = work_unit_coverage::stage_required_units(stage);
    for plan in &outcome.plans {
        required.extend(plan.work_unit_ids.iter().cloned());
    }
    let bundles = bundles_by_item(outcome, run_dir, stage)
        .into_values()
        .flatten()
        .collect();
    work_unit_gate::evaluate_required(run, stage, required, bundles)
}

fn item_coverage(
    run: &WorkflowRun,
    stage: &StageSpec,
    outcome: &CoordinatedOutcome,
    item_id: &str,
    bundles_by_item: &BTreeMap<String, Vec<EvidenceBundle>>,
) -> Option<WorkUnitCoverage> {
    let required = outcome
        .plans
        .iter()
        .find(|plan| plan.item_id == item_id)?
        .work_unit_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if required.is_empty() {
        return None;
    }
    let bundles = bundles_by_item.get(item_id).cloned().unwrap_or_default();
    Some(work_unit_gate::evaluate_required(
        run, stage, required, bundles,
    ))
}

fn bundles_by_item(
    outcome: &CoordinatedOutcome,
    run_dir: &Path,
    stage: &StageSpec,
) -> BTreeMap<String, Vec<EvidenceBundle>> {
    work_unit_coverage::bundles_from_agent_records(
        run_dir,
        &stage.id,
        outcome.item_status.keys().cloned().collect::<Vec<_>>(),
    )
}

fn manifest_accepts(status: &ManifestStatus) -> bool {
    matches!(
        status,
        ManifestStatus::Applied | ManifestStatus::IdempotentNoop
    )
}

fn item_error(
    status: &ManifestStatus,
    accepted: bool,
    item_coverage: Option<&WorkUnitCoverage>,
    stage_coverage: &WorkUnitCoverage,
) -> Option<String> {
    if accepted {
        return None;
    }
    if !manifest_accepts(status) {
        return Some(format!("{status:?}"));
    }
    if let Some(coverage) = item_coverage
        && coverage.verdict != CoverageVerdict::Accepted
    {
        return Some(work_unit_gate::error_summary(coverage));
    }
    Some(work_unit_gate::error_summary(stage_coverage))
}

fn failing_coverage<'a>(
    item_coverage: Option<&'a WorkUnitCoverage>,
    stage_coverage: &'a WorkUnitCoverage,
) -> Option<&'a WorkUnitCoverage> {
    match item_coverage {
        Some(coverage) if coverage.verdict != CoverageVerdict::Accepted => Some(coverage),
        _ if stage_coverage.verdict != CoverageVerdict::Accepted => Some(stage_coverage),
        other => other,
    }
}

#[allow(clippy::too_many_arguments)]
fn item_body(
    run_dir: &Path,
    stage: &StageSpec,
    outcome: &CoordinatedOutcome,
    item_id: &str,
    status: &ManifestStatus,
    accepted: bool,
    error: Option<&str>,
    item_coverage: Option<&WorkUnitCoverage>,
    coverage_artifact_key: &str,
) -> String {
    if accepted {
        let mut body = coordinated_accepted_item_body(outcome, item_id);
        append_coverage_summary(
            &mut body,
            run_dir,
            stage,
            item_id,
            item_coverage,
            coverage_artifact_key,
        );
        return body;
    }
    if !manifest_accepts(status) {
        return coordinated_item_body(
            run_dir,
            &stage.id,
            item_id,
            status,
            &coordinated_target_files(outcome, item_id),
        );
    }
    coverage_failure_body(
        run_dir,
        stage,
        item_id,
        error,
        item_coverage,
        coverage_artifact_key,
    )
}

fn append_coverage_summary(
    body: &mut String,
    run_dir: &Path,
    stage: &StageSpec,
    item_id: &str,
    item_coverage: Option<&WorkUnitCoverage>,
    coverage_artifact_key: &str,
) {
    let agent_output = run_dir
        .join("agent-outputs")
        .join(&stage.id)
        .join(format!("{item_id}.json"));
    let coverage = item_coverage
        .map(|coverage| {
            json!({
                "verdict": coverage.verdict.clone(),
                "required_work_units": coverage.required_work_units.clone(),
                "satisfied_work_units": coverage.satisfied_work_units.clone(),
                "blocked_work_units": coverage.blocked_work_units.clone(),
                "missing_work_units": coverage.missing_work_units.clone(),
                "stage_coverage_artifact": coverage_artifact_key,
                "original_agent_output": agent_output.display().to_string(),
            })
        })
        .unwrap_or_else(|| {
            json!({
                "verdict": "accepted",
                "stage_coverage_artifact": coverage_artifact_key,
                "original_agent_output": agent_output.display().to_string(),
            })
        });
    let rendered = serde_json::to_string_pretty(&coverage).unwrap_or_else(|_| "{}".into());
    body.push_str("\ncoverage:\n```json\n");
    body.push_str(&rendered);
    body.push_str("\n```\n");
}

fn coverage_failure_body(
    run_dir: &Path,
    stage: &StageSpec,
    item_id: &str,
    error: Option<&str>,
    item_coverage: Option<&WorkUnitCoverage>,
    coverage_artifact_key: &str,
) -> String {
    let agent_output = run_dir
        .join("agent-outputs")
        .join(&stage.id)
        .join(format!("{item_id}.json"));
    let manifest = run_dir
        .join("write-coordination/stages")
        .join(&stage.id)
        .join("manifests")
        .join(format!("{item_id}.json"));
    let coverage = item_coverage
        .and_then(|coverage| serde_json::to_string_pretty(coverage).ok())
        .unwrap_or_else(|| "{}".into());
    let remediation = item_coverage
        .map(remediation_items_json)
        .unwrap_or_else(|| "{}".into());
    format!(
        "# Coordinated Item `{item_id}`\n\n\
status: coverage_incomplete\n\
reason: {}\n\n\
## Evidence Paths\n\n\
- manifest: `{}`\n\
- agent_output: `{}`\n\
- stage_coverage_artifact: `{}`\n\n\
## Item Coverage\n\n```json\n{}\n```\n\n\
## Remediation Items\n\n```json\n{}\n```\n",
        error.unwrap_or("work-unit coverage incomplete"),
        manifest.display(),
        agent_output.display(),
        coverage_artifact_key,
        coverage,
        remediation
    )
}

fn remediation_items_json(coverage: &WorkUnitCoverage) -> String {
    let items = coverage
        .missing_work_units
        .iter()
        .map(|unit| {
            json!({
                "finding_id": format!("missing-work-unit:{unit}"),
                "work_unit_id": unit,
                "failure": "required work unit lacks accepted evidence",
                "required_fix": format!("Implement or produce verified already-complete evidence for work unit `{unit}`."),
                "required_tests": [
                    "Run the narrowest verification command that proves this work unit."
                ]
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&json!({ "items": items })).unwrap_or_else(|_| "{}".into())
}
