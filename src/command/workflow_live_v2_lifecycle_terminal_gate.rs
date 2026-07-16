use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support::{
    self as support, LifecycleContract,
};

const MAX_REROUTES_PER_BLOCKED_ID: usize = 2;

#[derive(Default)]
pub(super) struct TerminalGateState {
    pub(super) reroute_counts: BTreeMap<String, usize>,
    pub(super) pending_implementation_items: Vec<Value>,
    pub(super) completed_ids: BTreeSet<String>,
    pub(super) noop_reclassified_ids: BTreeSet<String>,
    pub(super) events: Vec<Value>,
}

#[derive(Debug, PartialEq)]
pub(super) enum TerminalGateDecision {
    Emit,
    Reroute(Value),
}

pub(super) fn decide(
    contract: &LifecycleContract<'_>,
    blocked_id: &str,
    inputs: &Value,
    state: &mut TerminalGateState,
) -> TerminalGateDecision {
    if !blocked_id.starts_with("blocked-") {
        return TerminalGateDecision::Emit;
    }

    let mut kinds = Vec::new();
    let mut work = Vec::new();
    let mut pending_implementation_items = Vec::new();
    let ready_noops = support::array(inputs.get("readyNoopItems"));
    let failed_noops = support::array(inputs.get("failedNoopProof"));
    if !ready_noops.is_empty() && !failed_noops.is_empty() {
        let route = super::workflow_live_v2_lifecycle_noop_routing::route_refuted_noops(
            contract,
            &ready_noops,
            &BTreeSet::new(),
            &failed_noops,
            &state.completed_ids,
            &mut state.noop_reclassified_ids,
        );
        if let super::workflow_live_v2_lifecycle_noop_routing::NoopProofExhaustionRoute::ScheduleImplementation(
            items,
        ) = route
        {
            kinds.push("refuted_noop");
            work.extend(items.iter().cloned());
            pending_implementation_items.extend(items);
        }
    }

    let attempted_retry_ids = attempted_retry_item_ids(inputs);
    let retry_items = keyed_arrays(inputs, &["retry_items", "retryItems"])
        .into_iter()
        .filter(|item| item_id(item).is_none_or(|id| !attempted_retry_ids.contains(id.as_str())))
        .collect::<Vec<_>>();
    if !retry_items.is_empty() {
        kinds.push("retry_items");
        work.extend(retry_items);
    }

    let remediation_items = actionable_inventory_items(inputs);
    if !remediation_items.is_empty() {
        kinds.push("actionable_remediation");
        work.extend(remediation_items);
    }

    if has_retryable_transport_failure(inputs) {
        kinds.push("transport_retry_budget");
    }

    kinds.sort_unstable();
    kinds.dedup();
    if kinds.is_empty() {
        return TerminalGateDecision::Emit;
    }

    let reroute_count = state
        .reroute_counts
        .entry(blocked_id.to_string())
        .or_default();
    if *reroute_count >= MAX_REROUTES_PER_BLOCKED_ID {
        state.events.push(serde_json::json!({
            "kind": "terminal-gate-reroute-exhausted",
            "blocked_id": blocked_id,
            "reroute_count": reroute_count,
            "schedulable_work_kinds": kinds,
            "reason": "bounded terminal-gate reroutes did not clear the schedulable work",
        }));
        return TerminalGateDecision::Emit;
    }
    *reroute_count += 1;
    state
        .pending_implementation_items
        .extend(pending_implementation_items);
    let event = serde_json::json!({
        "kind": "terminal-gate-reroute",
        "blocked_id": blocked_id,
        "reroute_count": reroute_count,
        "schedulable_work_kinds": kinds,
        "work": work.iter().map(compact_work_item).collect::<Vec<_>>(),
    });
    state.events.push(event.clone());
    TerminalGateDecision::Reroute(event)
}

pub(super) fn apply_pending_implementation_items(
    contract: &LifecycleContract<'_>,
    inventory: &Value,
    pending: Vec<Value>,
) -> Value {
    if pending.is_empty() {
        return inventory.clone();
    }
    let mut items = support::array(inventory.get("items"));
    for replacement in pending {
        let replacement_ids = contract
            .canonical_ids_for(&replacement)
            .into_iter()
            .collect::<BTreeSet<_>>();
        if let Some(index) = items.iter().position(|item| {
            contract
                .canonical_ids_for(item)
                .iter()
                .any(|id| replacement_ids.contains(id))
        }) {
            items[index] = replacement;
        } else {
            items.push(replacement);
        }
    }
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    object.insert("items".to_string(), Value::Array(items));
    contract.normalize_inventory(&Value::Object(object))
}

fn attempted_retry_item_ids(value: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    collect_attempted_retry_item_ids(value, &mut ids);
    ids
}

fn collect_attempted_retry_item_ids(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if object.get("kind").and_then(Value::as_str) == Some("verification-triage-retry") {
                for item in support::array(
                    object
                        .get("verificationPlan")
                        .and_then(|plan| plan.get("items")),
                ) {
                    if let Some(id) = item_id(&item) {
                        ids.insert(id);
                    }
                }
            }
            for child in object.values() {
                collect_attempted_retry_item_ids(child, ids);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_attempted_retry_item_ids(child, ids);
            }
        }
        _ => {}
    }
}

fn keyed_arrays(value: &Value, keys: &[&str]) -> Vec<Value> {
    let mut items = Vec::new();
    collect_keyed_arrays(value, keys, &mut items);
    items
}

fn collect_keyed_arrays(value: &Value, keys: &[&str], items: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if keys.contains(&key.as_str()) {
                    items.extend(support::array(Some(child)));
                }
                collect_keyed_arrays(child, keys, items);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_keyed_arrays(child, keys, items);
            }
        }
        _ => {}
    }
}

fn actionable_inventory_items(inputs: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    collect_actionable_inventory_items(inputs, &mut items);
    items
}

fn collect_actionable_inventory_items(value: &Value, items: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            for key in [
                "unscheduledFollowupInventory",
                "actionableRemediationInventory",
                "actionableInventory",
            ] {
                if let Some(inventory) = object.get(key)
                    && support::array(inventory.get("unresolved_issues")).is_empty()
                {
                    items.extend(support::array(inventory.get("items")));
                }
            }
            for child in object.values() {
                collect_actionable_inventory_items(child, items);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_actionable_inventory_items(child, items);
            }
        }
        _ => {}
    }
}

fn has_retryable_transport_failure(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            let retryable = object
                .get("failure_class")
                .or_else(|| object.get("failure_kind"))
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.to_ascii_lowercase().contains("transport"))
                && object
                    .get("transport_attempts")
                    .and_then(Value::as_u64)
                    .zip(object.get("max_transport_attempts").and_then(Value::as_u64))
                    .is_some_and(|(attempts, max)| attempts < max);
            retryable || object.values().any(has_retryable_transport_failure)
        }
        Value::Array(values) => values.iter().any(has_retryable_transport_failure),
        _ => false,
    }
}

fn item_id(item: &Value) -> Option<String> {
    item.get("item_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn compact_work_item(item: &Value) -> Value {
    serde_json::json!({
        "item_id": item.get("item_id").or_else(|| item.get("id")),
        "source_item_id": item.get("source_item_id"),
        "canonical_task_ids": item.get("canonical_task_ids"),
        "source_residual_gap_ids": item.get("source_residual_gap_ids"),
        "work_type": item.get("work_type"),
        "classification": item.get("classification"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::workflow_live::workflow_live_task_universe::{
        WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
    };

    fn universe() -> WorkflowV2TaskUniverse {
        WorkflowV2TaskUniverse {
            schema_version: "workflow-v2-task-universe-v1".to_string(),
            source_roots: Vec::new(),
            tasks: vec![WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-EX-001".to_string(),
                source_path: "tasks/TASK-EX-001.md".to_string(),
                acceptance_criteria: vec!["Evidence exists.".to_string()],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn blocked_report_reroutes_refuted_noop_without_targets() {
        let universe = universe();
        let contract = LifecycleContract {
            task_universe: &universe,
            target_repository_root: Some("/repo"),
        };
        let inputs = serde_json::json!({
            "readyNoopItems": [{
                "item_id": "noop-example",
                "work_type": "verified_noop",
                "canonical_task_ids": ["TASK-EX-001"],
                "dependency_ids": [],
                "acceptance_criteria": ["Evidence exists."],
                "target_files": [],
                "artifact_requirements": [],
            }],
            "failedNoopProof": [{
                "status": "needs_review",
                "canonical_task_ids": ["TASK-EX-001"],
                "residual_gaps": [{"id": "gap", "description": "evidence absent"}],
            }],
        });
        let mut state = TerminalGateState::default();

        assert!(matches!(
            decide(
                &contract,
                "blocked-noop-proof-failed-1",
                &inputs,
                &mut state
            ),
            TerminalGateDecision::Reroute(_)
        ));
        assert_eq!(state.pending_implementation_items.len(), 1);
    }

    #[test]
    fn terminal_gate_is_bounded_and_emits_when_no_work_exists() {
        let universe = universe();
        let contract = LifecycleContract {
            task_universe: &universe,
            target_repository_root: Some("/repo"),
        };
        let retry_inputs = serde_json::json!({
            "routing": {
                "retry_items": [{"item_id": "retry-one"}],
            },
        });
        let mut state = TerminalGateState::default();
        for _ in 0..MAX_REROUTES_PER_BLOCKED_ID {
            assert!(matches!(
                decide(
                    &contract,
                    "blocked-verification-failed-1",
                    &retry_inputs,
                    &mut state
                ),
                TerminalGateDecision::Reroute(_)
            ));
        }
        assert_eq!(
            decide(
                &contract,
                "blocked-verification-failed-1",
                &retry_inputs,
                &mut state
            ),
            TerminalGateDecision::Emit
        );
        assert_eq!(
            decide(
                &contract,
                "blocked-verification-failed-2",
                &serde_json::json!({}),
                &mut state
            ),
            TerminalGateDecision::Emit
        );
    }

    #[test]
    fn terminal_gate_evidence_compacts_schedulable_work() {
        let universe = universe();
        let contract = LifecycleContract {
            task_universe: &universe,
            target_repository_root: Some("/repo"),
        };
        let inputs = serde_json::json!({
            "routing": {
                "retry_items": [{
                    "item_id": "retry-one",
                    "canonical_task_ids": ["TASK-EX-001"],
                    "private_payload": "must-not-persist",
                }],
            },
        });
        let mut state = TerminalGateState::default();

        let TerminalGateDecision::Reroute(event) = decide(
            &contract,
            "blocked-verification-failed-1",
            &inputs,
            &mut state,
        ) else {
            panic!("retry work must reroute");
        };

        assert_eq!(event["work"][0]["item_id"], "retry-one");
        assert!(event.to_string().find("must-not-persist").is_none());
    }
}
