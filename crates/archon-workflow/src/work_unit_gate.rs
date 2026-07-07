use std::collections::BTreeSet;

use serde_json::Value;

use crate::run::WorkflowRun;
use crate::spec::StageSpec;
use crate::stage::source_input_hash;
use crate::work_unit_coverage::{self, EvidenceBundle, WorkUnitCoverage};

pub(crate) fn attempt_id(run: &WorkflowRun, stage: &StageSpec) -> String {
    let attempt = run
        .stages
        .get(&stage.id)
        .map(|state| state.attempt.max(1))
        .unwrap_or(1);
    format!("{}-attempt-{attempt}", stage.id)
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
