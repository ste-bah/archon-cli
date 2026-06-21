use std::collections::BTreeSet;

use serde_json::Value;

use crate::error::WorkflowResult;
use crate::persistence;
use crate::run::{ArtifactRef, WorkflowRun};
use crate::spec::StageSpec;
use crate::stage::source_input_hash;
use crate::store::WorkflowStore;
use crate::work_unit_coverage::{self, EvidenceBundle, WorkUnitCoverage};

pub(crate) fn attempt_id(run: &WorkflowRun, stage: &StageSpec) -> String {
    let attempt = run
        .stages
        .get(&stage.id)
        .map(|state| state.attempt.max(1))
        .unwrap_or(1);
    format!("{}-attempt-{attempt}", stage.id)
}

pub(crate) fn artifact_key(run: &WorkflowRun, stage: &StageSpec, base: &str) -> String {
    let attempt = run
        .stages
        .get(&stage.id)
        .map(|state| state.attempt.max(1))
        .unwrap_or(1);
    format!("{base}_attempt_{attempt}")
}

pub(crate) fn evaluate_required(
    run: &WorkflowRun,
    stage: &StageSpec,
    required: BTreeSet<String>,
    bundles: Vec<EvidenceBundle>,
) -> WorkUnitCoverage {
    let mut coverage = work_unit_coverage::evaluate(
        &run.id,
        &stage.id,
        &attempt_id(run, stage),
        required,
        bundles,
    );
    coverage.input_hash = source_input_hash(stage);
    coverage
}

pub(crate) fn evaluate_item_output(
    run: &WorkflowRun,
    stage: &StageSpec,
    item_id: &str,
    payload: &Value,
    body: &str,
) -> Option<WorkUnitCoverage> {
    let required = required_for_item(stage, payload);
    if required.is_empty() {
        return None;
    }
    let bundles = work_unit_coverage::bundles_from_output_body(body, Some(item_id.to_string()));
    Some(evaluate_required(run, stage, required, bundles))
}

pub(crate) fn evaluate_stage_output(
    run: &WorkflowRun,
    stage: &StageSpec,
    body: &str,
) -> Option<WorkUnitCoverage> {
    let required = work_unit_coverage::stage_required_units(stage);
    if required.is_empty() {
        return None;
    }
    let bundles = work_unit_coverage::bundles_from_output_body(body, Some(stage.id.clone()));
    Some(evaluate_required(run, stage, required, bundles))
}

pub(crate) fn evaluate_agent_records(
    run: &WorkflowRun,
    stage: &StageSpec,
    item_payloads: impl IntoIterator<Item = (String, Value)>,
    run_dir: &std::path::Path,
) -> Option<WorkUnitCoverage> {
    let items = item_payloads.into_iter().collect::<Vec<_>>();
    let mut required = work_unit_coverage::stage_required_units(stage);
    for (_, payload) in &items {
        required.extend(work_unit_coverage::item_required_units(payload));
    }
    if required.is_empty() {
        return None;
    }
    let item_ids = items
        .iter()
        .map(|(item_id, _)| item_id.clone())
        .collect::<Vec<_>>();
    let bundles = work_unit_coverage::bundles_from_agent_records(run_dir, &stage.id, item_ids)
        .into_values()
        .flatten()
        .collect();
    Some(evaluate_required(run, stage, required, bundles))
}

pub(crate) fn write_coverage_artifact(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    coverage: &WorkUnitCoverage,
    accepted: bool,
) -> WorkflowResult<ArtifactRef> {
    write_named_coverage_artifact(
        store,
        run,
        stage,
        "__work_unit_coverage",
        coverage,
        accepted,
    )
}

pub(crate) fn write_named_coverage_artifact(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    base_key: &str,
    coverage: &WorkUnitCoverage,
    accepted: bool,
) -> WorkflowResult<ArtifactRef> {
    let body = serde_json::to_string_pretty(coverage)
        .unwrap_or_else(|err| format!(r#"{{"serialization_error":"{err}"}}"#));
    let key = artifact_key(run, stage, base_key);
    persistence::write_attached_stage_artifact(store, run, stage, &key, "json", body, accepted)
}

pub(crate) fn error_summary(coverage: &WorkUnitCoverage) -> String {
    format!(
        "work-unit coverage {:?}: missing={:?}, blocked={:?}, satisfied={:?}",
        coverage.verdict,
        coverage.missing_work_units,
        coverage.blocked_work_units,
        coverage.satisfied_work_units
    )
}

pub(crate) fn required_for_item(stage: &StageSpec, payload: &Value) -> BTreeSet<String> {
    let item_units = work_unit_coverage::item_required_units(payload)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if item_units.is_empty() {
        work_unit_coverage::stage_required_units(stage)
    } else {
        item_units
    }
}
