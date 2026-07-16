use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support::{
    self as support, LifecycleContract,
};

#[derive(Debug, PartialEq)]
pub(super) enum NoopProofExhaustionRoute {
    ScheduleImplementation(Vec<Value>),
    Block,
}

pub(super) fn pin_noop_acceptance_criteria(
    contract: &LifecycleContract<'_>,
    items: &[Value],
) -> Vec<Value> {
    items
        .iter()
        .map(|item| {
            let criteria = authoritative_criteria_for_item(contract, item);
            let mut object = item.as_object().cloned().unwrap_or_default();
            object.insert(
                "acceptance_criteria".to_string(),
                serde_json::json!(
                    criteria
                        .iter()
                        .map(|(_, criterion)| criterion)
                        .collect::<Vec<_>>()
                ),
            );
            object.insert(
                "task_acceptance_criteria".to_string(),
                serde_json::json!(
                    criteria
                        .iter()
                        .map(|(task_id, criterion)| {
                            serde_json::json!({
                                "task_id": task_id,
                                "criterion": criterion,
                            })
                        })
                        .collect::<Vec<_>>()
                ),
            );
            Value::Object(object)
        })
        .collect()
}

pub(super) fn enforce_noop_acceptance_criteria(
    contract: &LifecycleContract<'_>,
    source_items: &[Value],
    outcomes: &[Value],
) -> Vec<Value> {
    outcomes
        .iter()
        .cloned()
        .map(|outcome| {
            if !support::outcome_accepted_or_noop(&outcome) {
                return outcome;
            }
            let Some(source_item) = source_items
                .iter()
                .find(|item| items_share_task(contract, item, &outcome))
            else {
                return refute_noop_criteria(
                    outcome,
                    &["no authoritative source item matched the accepted no-op outcome".into()],
                );
            };
            let expected = authoritative_criteria_for_item(contract, source_item);
            let results = acceptance_criteria_results(&outcome);
            let missing = expected
                .iter()
                .filter(|(task_id, criterion)| {
                    !results.iter().any(|result| {
                        result_matches_criterion(result, task_id, criterion)
                            && criterion_result_passed(result)
                    })
                })
                .map(|(task_id, criterion)| format!("{task_id}: {criterion}"))
                .collect::<Vec<_>>();
            if expected.is_empty() {
                return refute_noop_criteria(
                    outcome,
                    &["authoritative TASK file declared no parseable acceptance criteria".into()],
                );
            }
            if missing.is_empty() {
                outcome
            } else {
                refute_noop_criteria(outcome, &missing)
            }
        })
        .collect()
}

fn authoritative_criteria_for_item(
    contract: &LifecycleContract<'_>,
    item: &Value,
) -> Vec<(String, String)> {
    let task_ids = contract.canonical_ids_for(item);
    contract
        .task_universe
        .tasks
        .iter()
        .filter(|task| task_ids.contains(&task.canonical_task_id))
        .flat_map(|task| {
            task.acceptance_criteria
                .iter()
                .cloned()
                .map(|criterion| (task.canonical_task_id.clone(), criterion))
        })
        .collect()
}

fn items_share_task(contract: &LifecycleContract<'_>, left: &Value, right: &Value) -> bool {
    let left = contract
        .canonical_ids_for(left)
        .into_iter()
        .collect::<BTreeSet<_>>();
    contract
        .canonical_ids_for(right)
        .iter()
        .any(|id| left.contains(id))
}

fn acceptance_criteria_results(outcome: &Value) -> Vec<Value> {
    let mut results = Vec::new();
    for root in [
        Some(outcome),
        outcome.get("data"),
        outcome.get("result"),
        outcome.get("result").and_then(|result| result.get("data")),
    ]
    .into_iter()
    .flatten()
    {
        results.extend(support::array(root.get("acceptance_criteria_results")));
        results.extend(support::array(root.get("criterion_results")));
    }
    results
}

fn result_matches_criterion(result: &Value, task_id: &str, criterion: &str) -> bool {
    let result_criterion = result
        .get("criterion")
        .or_else(|| result.get("acceptance_criterion"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let result_task_id = result
        .get("task_id")
        .or_else(|| result.get("canonical_task_id"))
        .and_then(Value::as_str);
    result_criterion == criterion.trim()
        && result_task_id.is_none_or(|result_task_id| result_task_id == task_id)
}

fn criterion_result_passed(result: &Value) -> bool {
    result
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| {
            matches!(
                status.to_ascii_lowercase().as_str(),
                "accepted" | "complete" | "completed" | "pass" | "passed" | "satisfied"
            )
        })
        && support::present(result.get("evidence_refs"))
}

fn refute_noop_criteria(mut outcome: Value, missing: &[String]) -> Value {
    let gap = serde_json::json!({
        "id": "noop-acceptance-criteria-unsatisfied",
        "description": format!(
            "accepted/noop proof did not explicitly satisfy every authoritative TASK acceptance criterion: {}",
            missing.join(" | ")
        ),
        "severity": "blocking",
        "missing_criteria": missing,
    });
    let Some(object) = outcome.as_object_mut() else {
        return outcome;
    };
    object.insert("status".into(), Value::String("needs_review".into()));
    let mut gaps = support::array(object.get("residual_gaps"));
    gaps.push(gap.clone());
    object.insert("residual_gaps".into(), Value::Array(gaps));
    if let Some(result) = object.get_mut("result").and_then(Value::as_object_mut) {
        result.insert("status".into(), Value::String("needs_review".into()));
        let mut result_gaps = support::array(result.get("residual_gaps"));
        result_gaps.push(gap);
        result.insert("residual_gaps".into(), Value::Array(result_gaps));
    }
    outcome
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
                || !noop_item_is_implementable(&item)
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
                implementation_item(&item, ids, "inventory_contradiction", &gaps)
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
    reclassified_ids: &mut BTreeSet<String>,
) -> NoopProofExhaustionRoute {
    let semantic_refuted_ids = semantic_refuted_task_ids(contract, failed_outcomes);
    let single_item_semantic_fallback = semantic_refuted_ids.is_empty()
        && ready_noop_items.len() == 1
        && failed_outcomes.iter().any(semantic_noop_refutation);
    let mut implementation_items = Vec::new();
    for item in ready_noop_items {
        if !noop_item_is_implementable(item) {
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

fn noop_item_is_implementable(item: &Value) -> bool {
    support::present(item.get("target_files"))
        || support::present(item.get("artifact_requirements"))
}

fn implementation_item(
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
        serde_json::json!(canonical_task_ids),
    );
    object.insert(
        "work_type".to_string(),
        Value::String("implementation".to_string()),
    );
    object
        .entry("target_files".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
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
    Value::Object(object)
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

fn inventory_contradicts_noop(
    contract: &LifecycleContract<'_>,
    item: &Value,
    gaps: &[Value],
    task_coverage: &[Value],
) -> bool {
    let task_ids = contract.canonical_ids_for(item);
    task_coverage.iter().any(|coverage| {
        coverage_task_ids(coverage)
            .iter()
            .any(|id| task_ids.contains(id))
            && coverage
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| {
                    !matches!(
                        status.to_ascii_lowercase().as_str(),
                        "accepted" | "complete" | "completed" | "noop" | "verified_noop"
                    )
                })
    }) || gaps
        .iter()
        .any(|gap| gap_references_item(contract, gap, item, &task_ids))
}

fn coverage_task_ids(coverage: &Value) -> Vec<String> {
    let mut ids = support::strings_of(coverage.get("canonical_task_ids"));
    ids.extend(support::strings_of(coverage.get("task_ids")));
    for key in ["canonical_task_id", "task_id"] {
        if let Some(id) = coverage.get(key).and_then(Value::as_str) {
            ids.push(id.to_string());
        }
    }
    ids
}

fn gap_references_item(
    contract: &LifecycleContract<'_>,
    gap: &Value,
    item: &Value,
    task_ids: &[String],
) -> bool {
    let gap_text = serde_json::to_string(gap)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if task_ids
        .iter()
        .any(|task_id| gap_text.contains(&task_id.to_ascii_lowercase()))
    {
        return true;
    }
    let mut references = support::strings_of(item.get("artifact_requirements"));
    references.extend(support::strings_of(item.get("deliverable_contracts")));
    if references.into_iter().any(|reference| {
        reference_tokens(&reference)
            .into_iter()
            .any(|token| gap_text.contains(&token))
    }) {
        return true;
    }
    descriptor_tokens(contract, item, task_ids)
        .intersection(&lexical_tokens(&gap_text))
        .next()
        .is_some()
}

fn reference_tokens(reference: &str) -> Vec<String> {
    let lower = reference.to_ascii_lowercase();
    let mut tokens = Vec::new();
    if lower.len() >= 8 {
        tokens.push(lower);
    }
    if let Some(name) = Path::new(reference)
        .file_name()
        .and_then(|name| name.to_str())
    {
        let name = name.to_ascii_lowercase();
        if name.len() >= 8 && !name.contains('*') {
            tokens.push(name);
        }
    }
    tokens
}

fn descriptor_tokens(
    contract: &LifecycleContract<'_>,
    item: &Value,
    task_ids: &[String],
) -> BTreeSet<String> {
    let mut descriptors = Vec::new();
    if let Some(item_id) = item
        .get("item_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
    {
        descriptors.push(item_id.to_string());
    }
    descriptors.extend(
        contract
            .task_universe
            .tasks
            .iter()
            .filter(|task| task_ids.contains(&task.canonical_task_id))
            .filter_map(|task| task.title.clone()),
    );
    let mut tokens = BTreeSet::new();
    for descriptor in descriptors {
        let words = lexical_words(&descriptor);
        tokens.extend(words.iter().filter_map(|word| {
            (word.len() >= 4 && !descriptor_stopword(word)).then_some(word.clone())
        }));
        tokens.extend(words.windows(2).filter_map(|pair| {
            let acronym = pair
                .iter()
                .filter_map(|word| word.chars().next())
                .collect::<String>();
            (acronym.len() == 2 && !acronym.chars().any(|ch| ch.is_ascii_digit()))
                .then_some(acronym)
        }));
    }
    tokens
}

fn lexical_tokens(text: &str) -> BTreeSet<String> {
    lexical_words(text).into_iter().collect()
}

fn lexical_words(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

fn descriptor_stopword(word: &str) -> bool {
    matches!(
        word,
        "artifact"
            | "current"
            | "implementation"
            | "latest"
            | "noop"
            | "refuted"
            | "task"
            | "verified"
    )
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
