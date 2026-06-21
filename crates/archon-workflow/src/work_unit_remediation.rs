use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::error::WorkflowResult;
use crate::persistence;
use crate::run::WorkflowRun;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;
use crate::work_unit_coverage::WorkUnitCoverage;

#[derive(Debug, Clone, Default)]
pub(crate) struct UnitRepairSource {
    pub work_unit_ids: Vec<String>,
    pub target_files: Vec<String>,
    pub required_tests: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct MissingUnitRemediation {
    pub attempts_exhausted: bool,
}

pub(crate) fn source_from_payload(payload: &Value) -> UnitRepairSource {
    UnitRepairSource {
        work_unit_ids: crate::work_unit_coverage::item_required_units(payload),
        target_files: string_list(payload, &["target_files", "expected_target_files"]),
        required_tests: string_list(payload, &["required_tests", "tests", "verification"]),
    }
}

pub(crate) fn source_from_stage(stage: &StageSpec) -> UnitRepairSource {
    UnitRepairSource {
        work_unit_ids: crate::work_unit_coverage::stage_required_units(stage)
            .into_iter()
            .collect(),
        target_files: stage.expected_target_files.clone(),
        required_tests: Vec::new(),
    }
}

pub(crate) fn source_from_plan(
    work_unit_ids: Vec<String>,
    target_files: Vec<String>,
) -> UnitRepairSource {
    UnitRepairSource {
        work_unit_ids,
        target_files,
        required_tests: Vec::new(),
    }
}

pub(crate) fn write_missing_unit_items(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    coverage: &WorkUnitCoverage,
    mut sources: Vec<UnitRepairSource>,
    policy_max_attempts: u32,
) -> WorkflowResult<MissingUnitRemediation> {
    if coverage.missing_work_units.is_empty() {
        return Ok(MissingUnitRemediation {
            attempts_exhausted: false,
        });
    }
    let attempt = stage_attempt(run, stage);
    let max_attempts = max_attempts(stage, policy_max_attempts);
    let exhausted = attempt > max_attempts;
    sources.push(source_from_stage(stage));
    let items = if exhausted {
        Vec::new()
    } else {
        coverage
            .missing_work_units
            .iter()
            .map(|unit| item_for_unit(unit, &sources, coverage))
            .collect::<Vec<_>>()
    };
    let body = serde_json::to_string_pretty(&json!({
        "schema": "archon.workflow.missing_work_unit_remediation.v1",
        "coverage_attempt_id": coverage.attempt_id,
        "stage_id": stage.id,
        "self_heal_attempt": attempt,
        "max_self_heal_attempts": max_attempts,
        "attempts_exhausted": exhausted,
        "blocked_work_units": if exhausted { coverage.missing_work_units.clone() } else { Vec::new() },
        "items": items,
    }))?;
    let key = crate::work_unit_gate::artifact_key(run, stage, "__missing_work_unit_remediation");
    persistence::write_attached_stage_artifact(store, run, stage, &key, "json", body, false)?;
    Ok(MissingUnitRemediation {
        attempts_exhausted: exhausted,
    })
}

fn stage_attempt(run: &WorkflowRun, stage: &StageSpec) -> u32 {
    run.stages
        .get(&stage.id)
        .map(|state| state.attempt.max(1))
        .unwrap_or(1)
}

fn max_attempts(stage: &StageSpec, policy_max_attempts: u32) -> u32 {
    stage
        .extra
        .get("missing_unit_remediation_max_attempts")
        .or_else(|| stage.input.get("missing_unit_remediation_max_attempts"))
        .and_then(serde_json::Value::as_u64)
        .map(|value| value.max(1) as u32)
        .unwrap_or_else(|| policy_max_attempts.max(1))
}

fn item_for_unit(unit: &str, sources: &[UnitRepairSource], coverage: &WorkUnitCoverage) -> Value {
    let target_files = values_for_unit(unit, sources, |source| &source.target_files);
    let required_tests = values_for_unit(unit, sources, |source| &source.required_tests);
    json!({
        "finding_id": format!("missing-work-unit:{unit}"),
        "work_unit_id": unit,
        "task_id": unit,
        "related_task_id": unit,
        "target_files": target_files,
        "required_tests": required_tests,
        "failure": "required work unit has no accepted implementation evidence",
        "required_fix": format!("Implement work unit `{unit}` or provide evidence-backed already-complete/no-op proof."),
        "required_evidence": ["file", "artifact", "command", "test", "review"],
        "coverage_verdict": format!("{:?}", coverage.verdict).to_ascii_lowercase(),
    })
}

fn values_for_unit(
    unit: &str,
    sources: &[UnitRepairSource],
    field: fn(&UnitRepairSource) -> &Vec<String>,
) -> Vec<String> {
    let mut out = BTreeSet::new();
    for source in sources {
        if source.work_unit_ids.iter().any(|id| id == unit) {
            out.extend(
                field(source)
                    .iter()
                    .filter(|v| !v.trim().is_empty())
                    .cloned(),
            );
        }
    }
    out.into_iter().collect()
}

fn string_list(payload: &Value, keys: &[&str]) -> Vec<String> {
    let mut out = BTreeMap::new();
    for key in keys {
        match payload.get(*key) {
            Some(Value::String(value)) if !value.trim().is_empty() => {
                out.insert(value.trim().to_string(), ());
            }
            Some(Value::Array(values)) => {
                for value in values.iter().filter_map(Value::as_str) {
                    if !value.trim().is_empty() {
                        out.insert(value.trim().to_string(), ());
                    }
                }
            }
            _ => {}
        }
    }
    out.into_keys().collect()
}
