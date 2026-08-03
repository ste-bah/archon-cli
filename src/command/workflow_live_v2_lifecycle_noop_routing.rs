use std::collections::BTreeSet;

use serde_json::Value;

use archon_workflow::generated_lifecycle_support::{self as support, LifecycleContract};

#[path = "workflow_live_v2_lifecycle_noop_acceptance.rs"]
mod acceptance;
pub(super) use acceptance::{enforce_noop_acceptance_criteria, pin_noop_acceptance_criteria};

#[path = "workflow_live_v2_lifecycle_noop_matching.rs"]
mod matching;
use matching::inventory_contradicts_noop;

#[derive(Debug, PartialEq)]
pub(super) enum NoopProofExhaustionRoute {
    ScheduleImplementation(Vec<Value>),
    Block,
}

pub(super) fn reclassify_inventory_contradicted_noops(
    contract: &LifecycleContract<'_>,
    inventory: &Value,
) -> (Value, BTreeSet<String>) {
    let gaps = inventory_values(inventory, "residual_gaps");
    let task_coverage = inventory_values(inventory, "task_coverage");
    let mut reclassified_ids = BTreeSet::new();
    let items = support::array(inventory.get("items"))
        .into_iter()
        .map(|item| {
            if support::work_type_for(&item) != "verified_noop"
                || !noop_item_has_authoritative_acceptance_criteria(contract, &item)
                || !inventory_contradicts_noop(contract, &item, &gaps, &task_coverage)
            {
                return item;
            }
            let ids = contract
                .canonical_ids_for(&item)
                .into_iter()
                .filter(|id| reclassified_ids.insert(id.clone()))
                .collect::<Vec<_>>();
            if ids.is_empty() {
                item
            } else {
                implementation_item(contract, &item, ids, "inventory_contradiction", &gaps)
            }
        })
        .collect::<Vec<_>>();
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    object.insert("items".to_string(), Value::Array(items));
    (Value::Object(object), reclassified_ids)
}

pub(super) fn route_refuted_noops(
    contract: &LifecycleContract<'_>,
    ready_noop_items: &[Value],
    accepted_ids: &BTreeSet<String>,
    failed_outcomes: &[Value],
    completed_ids: &BTreeSet<String>,
    reclassified_ids: &mut BTreeSet<String>,
) -> NoopProofExhaustionRoute {
    let semantic_refuted_ids = semantic_refuted_task_ids(contract, failed_outcomes);
    let single_item_semantic_fallback = semantic_refuted_ids.is_empty()
        && ready_noop_items.len() == 1
        && failed_outcomes.iter().any(semantic_noop_refutation);
    let mut implementation_items = Vec::new();
    for item in ready_noop_items {
        if !noop_item_is_implementable(contract, item, completed_ids) {
            continue;
        }
        let ids = contract
            .canonical_ids_for(item)
            .into_iter()
            .filter(|id| {
                !accepted_ids.contains(id)
                    && !reclassified_ids.contains(id)
                    && (single_item_semantic_fallback || semantic_refuted_ids.contains(id))
            })
            .collect::<Vec<_>>();
        if ids.is_empty() {
            continue;
        }
        reclassified_ids.extend(ids.iter().cloned());
        implementation_items.push(implementation_item(
            contract,
            item,
            ids,
            "bounded_noop_proof_refutation",
            failed_outcomes,
        ));
    }
    if implementation_items.is_empty() {
        NoopProofExhaustionRoute::Block
    } else {
        NoopProofExhaustionRoute::ScheduleImplementation(implementation_items)
    }
}

fn semantic_refuted_task_ids(
    contract: &LifecycleContract<'_>,
    records: &[Value],
) -> BTreeSet<String> {
    let universe = contract.canonical_universe();
    let mut ids = BTreeSet::new();
    for record in records
        .iter()
        .filter(|record| semantic_noop_refutation(record))
    {
        collect_explicit_task_ids(record, &mut ids);
    }
    ids.retain(|id| universe.contains(id));
    ids
}

fn semantic_noop_refutation(record: &Value) -> bool {
    if contains_failure_class(record, &["transport", "infrastructure"]) {
        return false;
    }
    contains_nonempty_array(record, "residual_gaps")
        || contains_truthy_key(record, "proof_gap")
        || contains_failed_task_coverage(record)
}

fn contains_failure_class(value: &Value, needles: &[&str]) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (matches!(key.as_str(), "failure_class" | "failure_kind")
                && value.as_str().is_some_and(|class| {
                    let class = class.to_ascii_lowercase();
                    needles.iter().any(|needle| class.contains(needle))
                }))
                || contains_failure_class(value, needles)
        }),
        Value::Array(values) => values
            .iter()
            .any(|value| contains_failure_class(value, needles)),
        _ => false,
    }
}

fn contains_nonempty_array(value: &Value, target_key: &str) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == target_key && value.as_array().is_some_and(|items| !items.is_empty()))
                || contains_nonempty_array(value, target_key)
        }),
        Value::Array(values) => values
            .iter()
            .any(|value| contains_nonempty_array(value, target_key)),
        _ => false,
    }
}

fn contains_truthy_key(value: &Value, target_key: &str) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == target_key && support::present(Some(value)))
                || contains_truthy_key(value, target_key)
        }),
        Value::Array(values) => values
            .iter()
            .any(|value| contains_truthy_key(value, target_key)),
        _ => false,
    }
}

fn contains_failed_task_coverage(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == "task_coverage"
                && support::array(Some(value)).iter().any(|coverage| {
                    coverage
                        .get("status")
                        .and_then(Value::as_str)
                        .is_some_and(|status| {
                            !matches!(
                                status.to_ascii_lowercase().as_str(),
                                "accepted" | "complete" | "completed" | "noop" | "verified_noop"
                            )
                        })
                }))
                || contains_failed_task_coverage(value)
        }),
        Value::Array(values) => values.iter().any(contains_failed_task_coverage),
        _ => false,
    }
}

fn collect_explicit_task_ids(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for key in [
                "canonical_task_ids",
                "task_ids",
                "canonical_task_id",
                "task_id",
            ] {
                if let Some(value) = object.get(key) {
                    ids.extend(support::strings_of(Some(value)));
                    if let Some(id) = value.as_str() {
                        ids.insert(id.to_string());
                    }
                }
            }
            for child in object.values() {
                collect_explicit_task_ids(child, ids);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_explicit_task_ids(child, ids);
            }
        }
        _ => {}
    }
}

fn noop_item_is_implementable(
    contract: &LifecycleContract<'_>,
    item: &Value,
    completed_ids: &BTreeSet<String>,
) -> bool {
    noop_item_has_authoritative_acceptance_criteria(contract, item)
        && contract
            .dependency_ids_for(item)
            .iter()
            .all(|dependency_id| completed_ids.contains(dependency_id))
}

fn noop_item_has_authoritative_acceptance_criteria(
    contract: &LifecycleContract<'_>,
    item: &Value,
) -> bool {
    let task_ids = contract.canonical_ids_for(item);
    !task_ids.is_empty()
        && contract
            .task_universe
            .tasks
            .iter()
            .filter(|task| task_ids.contains(&task.canonical_task_id))
            .all(|task| !task.acceptance_criteria.is_empty())
}

fn implementation_item(
    contract: &LifecycleContract<'_>,
    item: &Value,
    canonical_task_ids: Vec<String>,
    source: &str,
    failure_records: &[Value],
) -> Value {
    let mut object = item.as_object().cloned().unwrap_or_default();
    let original_id = item
        .get("item_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("refuted-noop");
    let implementation_id = format!("implementation-refuted-{original_id}");
    let refuted_claim = object.remove("noop_proof").unwrap_or(Value::Null);
    object.remove("noop_proof_refs");
    object.insert(
        "item_id".to_string(),
        Value::String(implementation_id.clone()),
    );
    object.insert("id".to_string(), Value::String(implementation_id));
    object.insert(
        "source_item_id".to_string(),
        Value::String(original_id.to_string()),
    );
    object.insert(
        "canonical_task_ids".to_string(),
        serde_json::json!(&canonical_task_ids),
    );
    object.insert(
        "work_type".to_string(),
        Value::String("implementation".to_string()),
    );
    object
        .entry("target_files".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let mut artifact_requirements = support::strings_of(object.get("artifact_requirements"));
    for task in &contract.task_universe.tasks {
        if canonical_task_ids.contains(&task.canonical_task_id) {
            artifact_requirements.extend(task.artifact_requirements.iter().cloned());
        }
    }
    artifact_requirements.sort();
    artifact_requirements.dedup();
    object.insert(
        "artifact_requirements".to_string(),
        serde_json::json!(artifact_requirements),
    );
    if !support::present(object.get("focused_verification")) {
        object.insert(
            "focused_verification".to_string(),
            object
                .get("acceptance_criteria")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
        );
    }
    if !support::present(object.get("expected_evidence")) {
        object.insert(
            "expected_evidence".to_string(),
            object
                .get("acceptance_criteria")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
        );
    }
    let compact_failures = compact_failure_records(failure_records);
    object.insert(
        "required_fix".to_string(),
        serde_json::json!(failure_descriptions(&compact_failures)),
    );
    object.insert("failure_evidence".to_string(), compact_failures);
    object.insert(
        "noop_reclassification".to_string(),
        serde_json::json!({
            "count": 1,
            "source": source,
            "refuted_claim": refuted_claim,
        }),
    );
    contract.normalize_item(&Value::Object(object))
}

fn compact_failure_records(records: &[Value]) -> Value {
    Value::Array(
        records
            .iter()
            .map(|record| {
                let result = record.get("result").unwrap_or(record);
                serde_json::json!({
                    "item_id": record.get("item_id").or_else(|| record.get("id")),
                    "status": record.get("status").or_else(|| result.get("status")),
                    "summary": result.get("summary"),
                    "residual_gaps": result.get("residual_gaps"),
                    "task_coverage": result.get("task_coverage"),
                })
            })
            .collect(),
    )
}

fn failure_descriptions(records: &Value) -> Vec<String> {
    let mut descriptions = support::array(Some(records))
        .into_iter()
        .flat_map(|record| support::array(record.get("residual_gaps")))
        .filter_map(|gap| {
            gap.get("description")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    descriptions.sort();
    descriptions.dedup();
    if descriptions.is_empty() {
        descriptions.push(
            "Satisfy the acceptance criteria refuted by bounded no-op proof verification."
                .to_string(),
        );
    }
    descriptions
}

fn inventory_values(inventory: &Value, key: &str) -> Vec<Value> {
    let mut values = Vec::new();
    for root in [
        Some(inventory),
        inventory.get("result"),
        inventory.get("data"),
        inventory
            .get("result")
            .and_then(|result| result.get("data")),
    ]
    .into_iter()
    .flatten()
    {
        values.extend(support::array(root.get(key)));
    }
    values
}

#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_noop_routing_tests.rs"]
mod tests;
