//! Do not dispatch a write branch whose declared dependencies have not landed.
//!
//! The write planner orders waves by `dependency_ids`, but ordering is not
//! satisfaction: when a wave item ends without acceptance, the next wave used
//! to run anyway against a baseline that lacked what its tasks depend on. A
//! task that consumes another task's new API then implements against nothing.
//! Here a branch whose dependency has no accepted or no-op outcome in this run
//! is held back with a typed outcome the authored script's remediation loop
//! retries in canonical order, once the dependency has landed.
use std::collections::BTreeMap;

use super::errors::sanitize_v2_path_segment;
use crate::generated_contract::canonical_task_ids_from_generated_value;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::{
    BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status,
};

/// Task ids with an accepted or no-op branch outcome recorded in this run.
pub(crate) fn landed_task_ids(outcomes: &[WorkflowV2BranchOutcome]) -> Vec<String> {
    let mut landed = Vec::new();
    for outcome in outcomes {
        let Some(result) = outcome.result.as_ref() else {
            continue;
        };
        if !matches!(
            result.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ) {
            continue;
        }
        if let Some(ids) = result
            .data
            .get("canonical_task_ids")
            .and_then(|v| v.as_array())
        {
            landed.extend(ids.iter().filter_map(|id| id.as_str().map(str::to_string)));
        }
    }
    landed.sort();
    landed.dedup();
    landed
}

/// For each branch input, the dependency ids that are in the universe and have
/// not landed. A dependency the universe does not know is not this run's to
/// satisfy and never blocks.
pub(crate) fn unmet_dependencies(
    branches: &[(String, serde_json::Value)],
    universe: Option<&WorkflowV2TaskUniverse>,
    landed: &[String],
) -> BTreeMap<String, Vec<String>> {
    let Some(universe) = universe else {
        return BTreeMap::new();
    };
    let mut blocked = BTreeMap::new();
    for (item_id, input) in branches {
        let source = input.get("item").unwrap_or(input);
        let mut missing = Vec::new();
        for task_id in canonical_task_ids_from_generated_value(source, Some(universe)) {
            let Some(task) = universe
                .tasks
                .iter()
                .find(|t| t.canonical_task_id == task_id)
            else {
                continue;
            };
            for dep in &task.dependency_ids {
                let known = universe.tasks.iter().any(|t| &t.canonical_task_id == dep);
                if known && !landed.iter().any(|l| l == dep) && !missing.contains(dep) {
                    missing.push(dep.clone());
                }
            }
        }
        if !missing.is_empty() {
            blocked.insert(item_id.clone(), missing);
        }
    }
    blocked
}

/// The outcome a held-back branch records: review, not failure, keyed for the
/// script to retry once the named tasks have landed.
pub(crate) fn blocked_on_dependency_result(
    item_id: &str,
    input: &serde_json::Value,
    universe: Option<&WorkflowV2TaskUniverse>,
    missing: &[String],
) -> WorkflowV2Result {
    let source = input.get("item").unwrap_or(input);
    let canonical_task_ids = canonical_task_ids_from_generated_value(source, universe);
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!(
            "write branch '{item_id}' was not dispatched: its dependencies {} have no accepted outcome in this run",
            missing.join(", ")
        ),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "branch held back until its dependencies land; no worktree was created and no agent ran",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "blocked_on_dependency_{}",
            sanitize_v2_path_segment(item_id)
        ),
        description: format!("waiting for {}", missing.join(", ")),
        severity: Some("review".to_string()),
    });
    result.data = serde_json::json!({
        "branch_id": item_id,
        "item_id": item_id,
        "canonical_task_ids": canonical_task_ids,
        "blocked_on_dependency": missing,
        "failure_kind": BranchFailureKind::Contract,
    });
    result
}

/// Split a wave into the assignments that may run and the typed outcomes of
/// those held back. Held-back outcomes are saved like any branch outcome so
/// status, resume and the script's remediation loop all see them.
pub(super) fn hold_back_unmet(
    ctx: &super::worktree_wave::WorktreePlanRunContext<'_>,
    wave: &crate::v2::write_mode::WorkflowV2WriteWave,
    branches: &[crate::WorkflowV2FanoutItem],
) -> crate::WorkflowResult<(
    crate::v2::write_mode::WorkflowV2WriteWave,
    Vec<WorkflowV2Result>,
)> {
    let landed = landed_task_ids(&ctx.v2_store.load_branch_outcomes()?);
    let inputs: Vec<(String, serde_json::Value)> = wave
        .assignments
        .iter()
        .filter_map(|assignment| {
            branches
                .iter()
                .find(|branch| branch.id == assignment.item_id)
                .map(|branch| (branch.id.clone(), branch.input.clone()))
        })
        .collect();
    let blocked = unmet_dependencies(&inputs, ctx.task_universe, &landed);
    if blocked.is_empty() {
        return Ok((wave.clone(), Vec::new()));
    }
    let mut held = Vec::new();
    for (item_id, missing) in &blocked {
        let Some(branch) = branches.iter().find(|branch| &branch.id == item_id) else {
            continue;
        };
        let mut result =
            blocked_on_dependency_result(item_id, &branch.input, ctx.task_universe, missing);
        super::contract::tag_branch_result(&mut result, item_id);
        super::contract::save_write_branch_outcome(
            ctx.v2_store,
            &ctx.execution.call.id,
            item_id,
            &branch.role,
            Some(branch.input_hash()),
            &result,
        )?;
        held.push(result);
    }
    let runnable = crate::v2::write_mode::WorkflowV2WriteWave {
        assignments: wave
            .assignments
            .iter()
            .filter(|assignment| !blocked.contains_key(&assignment.item_id))
            .cloned()
            .collect(),
    };
    Ok((runnable, held))
}

#[cfg(test)]
#[path = "dependency_gate_tests.rs"]
mod tests;
