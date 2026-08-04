//! Host-side stamping of fan-out branches from the authoritative task universe.

use crate::task_universe::WorkflowV2TaskUniverse;

use super::WorkflowV2FanoutItem;

/// Collect `item_id -> (artifact_root, deliverable_contract)` for every fanout
/// item that declared a contract, so the host can verify the declared deliverable
/// itself rather than trusting the branch's self-reported verification.
///
/// The root is the item's stamped project artifact root (contract paths are
/// declared relative to it); items lacking either a contract or a root are
/// skipped — nothing is invented. Domain-agnostic: the contract's own content
/// decides what gets checked.
pub fn declared_contracts_by_item(
    items: &[WorkflowV2FanoutItem],
) -> std::collections::BTreeMap<String, (String, Vec<serde_json::Value>)> {
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
        contracts.insert(item.id.clone(), (root.to_string(), declared));
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
fn branch_canonical_task_ids(input: &serde_json::Value) -> Vec<String> {
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
