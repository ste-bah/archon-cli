//! Host-side stamping of fan-out branches from the authoritative task universe.

use crate::generated_contract::canonical_task_ids_from_generated_value;
use crate::task_universe::WorkflowV2TaskUniverse;

use super::deliverable_contract::ContractRoots;
use super::{WorkflowV2FanoutItem, WorkflowV2Result};

/// Every branch outcome names the canonical tasks its item owns. Failure
/// results already do; an accepted result's `data` is the agent's own, so the
/// ids are stamped from the host's item input before the outcome is saved.
///
/// Born in the write path (TD-058: without it the dependency gate saw nothing
/// landed and held every later wave). Shared since Obs-22, run wf-719ff3b0:
/// read-only review branches `adversarial-review-map-4` and `-8` ran fully,
/// returned zero findings, and left `data.canonical_task_ids` null because the
/// reviewer named its task only in prose. Nothing downstream could then tell
/// "reviewed, nothing to report" from "never reviewed", and the reducer
/// reported both tasks as lacking a review. The stamp is the host's item
/// input, so it holds whatever the agent chose to echo; a non-empty value the
/// agent returned is never overwritten.
pub fn stamp_canonical_task_ids(
    result: &mut WorkflowV2Result,
    input: &serde_json::Value,
    universe: Option<&WorkflowV2TaskUniverse>,
) {
    let source = input.get("item").unwrap_or(input);
    let ids = canonical_task_ids_from_generated_value(source, universe);
    if ids.is_empty() {
        return;
    }
    if !result.data.is_object() {
        result.data = serde_json::json!({});
    }
    let present = result
        .data
        .get("canonical_task_ids")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    if !present {
        result.data["canonical_task_ids"] = serde_json::json!(ids);
    }
}

/// Collect `item_id -> (roots, deliverable_contracts)` for every fanout item
/// that declared a contract, so the host can verify the declared deliverable
/// itself rather than trusting the branch's self-reported verification.
///
/// The roots are the item's stamped project artifact root first — contract
/// paths are declared relative to it — then `target_repository_root` when the
/// caller knows one (Issue-22: a deliverable that lives in the repository is
/// not missing because the project is a different directory). Items lacking
/// either a contract or a project root are skipped — nothing is invented.
/// Domain-agnostic: the contract's own content decides what gets checked.
pub fn declared_contracts_by_item(
    items: &[WorkflowV2FanoutItem],
    target_repository_root: Option<&str>,
) -> std::collections::BTreeMap<String, (ContractRoots, Vec<serde_json::Value>)> {
    let mut contracts = std::collections::BTreeMap::new();
    for item in items {
        // `deliverable_contract` is the decomposed path's singular stamp (one
        // verification item per contract); `deliverable_contracts` is the v3
        // stamp, where one verification item covers a whole task and must
        // enforce every contract that task declared.
        let declared: Vec<serde_json::Value> = item
            .input
            .get("deliverable_contracts")
            .and_then(serde_json::Value::as_array)
            .map(|values| values.iter().filter(|v| v.is_object()).cloned().collect())
            .or_else(|| {
                item.input
                    .get("deliverable_contract")
                    .filter(|contract| contract.is_object())
                    .map(|contract| vec![contract.clone()])
            })
            .unwrap_or_default();
        if declared.is_empty() {
            continue;
        }
        let root = item
            .input
            .get("_workflow_project_artifact_policy")
            .and_then(|policy| policy.get("project_root"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                item.input
                    .get("project_artifact_root")
                    .and_then(serde_json::Value::as_str)
            });
        let Some(root) = root else {
            continue;
        };
        contracts.insert(
            item.id.clone(),
            (ContractRoots::new(root, target_repository_root), declared),
        );
    }
    contracts
}

/// Stamp each read-only branch with the deliverable contracts its task declared,
/// looked up in the AUTHORITATIVE task universe by canonical task id.
///
/// Without this the host contract verifier is dead code in the lifecycle we
/// actually run. Contracts were only ever attached by `prepare_verification_items`,
/// which belongs to the decomposed path; the v3 authored prelude builds its own
/// verification item and calls `w.parallel` directly, so no item carried a
/// contract, `declared_contracts_by_item` found nothing, and
/// `enforce_declared_contracts` early-returned every time. Observed live: zero
/// results across a full run mentioned declared_contract_verification, and a
/// coverage task was accepted over dozens of fabricated cells that the
/// verifier's own predicates reject instantly.
///
/// Host-side and universe-sourced on purpose: the authored script cannot omit,
/// weaken or invent a contract. Generic — the engine matches by task id and
/// never reads what the contract contains.
pub fn stamp_declared_contracts_from_universe(
    mut items: Vec<WorkflowV2FanoutItem>,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> Vec<WorkflowV2FanoutItem> {
    let Some(universe) = task_universe else {
        return items;
    };
    for item in &mut items {
        // Already stamped by the decomposed path: leave it alone.
        if item.input.get("deliverable_contract").is_some()
            || item.input.get("deliverable_contracts").is_some()
        {
            continue;
        }
        let claimed = branch_canonical_task_ids(&item.input);
        if claimed.is_empty() {
            continue;
        }
        let declared: Vec<serde_json::Value> = universe
            .tasks
            .iter()
            .filter(|task| claimed.iter().any(|id| id == &task.canonical_task_id))
            .flat_map(|task| task.deliverable_contracts.iter())
            .filter_map(|contract| serde_json::to_value(contract).ok())
            .collect();
        if declared.is_empty() {
            continue;
        }
        if let Some(object) = item.input.as_object_mut() {
            object.insert(
                "deliverable_contracts".to_string(),
                serde_json::Value::Array(declared),
            );
        }
    }
    items
}

/// Stamp each read-only branch with the tools its task declared it needs,
/// looked up in the AUTHORITATIVE task universe by canonical task id.
///
/// Write branches have always got this (`stamp_required_tools_from_universe`),
/// and the decomposed path stamps it onto its verification items too — but the
/// v3 authored path builds its own verification items and never did. A task
/// whose acceptance requires live tool invocations then had a verifier that
/// could not invoke them: observed live as "this stage only had
/// Read/Grep/Glob/Bash and could not call the required tools", against a task
/// whose acceptance criteria demand exactly those calls. Unverifiable by
/// construction, three attempts each, no action any agent could take.
///
/// Universe-sourced so an authored script cannot grant itself tools; this only
/// mirrors what the task file already declares. Read-only refers to REPO
/// writes — it does not mean a verifier must be blind to the systems the task
/// is about.
pub fn stamp_required_tools_from_universe(
    mut items: Vec<WorkflowV2FanoutItem>,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> Vec<WorkflowV2FanoutItem> {
    let Some(universe) = task_universe else {
        return items;
    };
    for item in &mut items {
        let claimed = branch_canonical_task_ids(&item.input);
        if claimed.is_empty() {
            continue;
        }
        let tools: std::collections::BTreeSet<String> = universe
            .tasks
            .iter()
            .filter(|task| claimed.iter().any(|id| id == &task.canonical_task_id))
            .flat_map(|task| task.required_tools.iter().cloned())
            .collect();
        if tools.is_empty() {
            continue;
        }
        if let Some(object) = item
            .input
            .get_mut("item")
            .and_then(serde_json::Value::as_object_mut)
        {
            object.insert(
                "required_tools".to_string(),
                serde_json::json!(tools.into_iter().collect::<Vec<_>>()),
            );
        }
    }
    items
}

/// Canonical task ids claimed by a branch, from either nesting the item
/// builders produce.
pub fn branch_canonical_task_ids(input: &serde_json::Value) -> Vec<String> {
    input
        .get("item")
        .and_then(|item| item.get("canonical_task_ids"))
        .or_else(|| input.get("canonical_task_ids"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
#[path = "branch_stamping_tests.rs"]
mod tests;
