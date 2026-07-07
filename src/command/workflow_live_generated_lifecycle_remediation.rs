//! Remediation and review-remediation inventory helpers for the Rust
//! decomposed-PRD lifecycle — faithful ports of
//! `workflow_live_generated_scaffold_remediation.js` and the review helpers in
//! `workflow_live_generated_scaffold_body_b.js`.

use std::collections::BTreeSet;

use serde_json::Value;

use super::workflow_live_generated_lifecycle_support::{
    LifecycleContract, array, inventory_source_issues, present, raw_strings, strings_of,
};

fn issue(kind: &str, item: &Value, field: &str, message: &str) -> Value {
    serde_json::json!({
        "kind": kind,
        "field": field,
        "message": message,
        "item_id": item.get("item_id").or_else(|| item.get("id")),
        "canonical_task_ids": array(item.get("canonical_task_ids")),
    })
}

/// JS `generatedContractTargetFileIssue` semantics live in the Rust contract
/// twin (`target_file_issue`); re-exposed here for remediation items.
fn target_file_issue_message(
    contract: &LifecycleContract<'_>,
    target: &str,
) -> Option<&'static str> {
    super::workflow_live_generated_contract::lifecycle_target_file_issue(
        target,
        contract.target_repository_root,
    )
}

/// JS `remediationItemIssues`.
fn remediation_item_issues(contract: &LifecycleContract<'_>, item: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    if !(item.get("item_id").is_some() || item.get("id").is_some()) {
        issues.push(issue(
            "inventory_shape_repair",
            item,
            "item_id",
            "remediation item is missing item_id/id",
        ));
    }
    if contract.canonical_ids_for(item).is_empty() {
        issues.push(issue(
            "task_universe_reconcile",
            item,
            "canonical_task_ids",
            "remediation item must use canonical taskUniverse IDs",
        ));
    }
    for dep in contract.invalid_dependency_ids_for(item) {
        issues.push(issue(
            "task_universe_reconcile",
            item,
            "dependency_ids",
            &format!("remediation dependency is not canonical: {dep}"),
        ));
    }
    for required in [
        "source_item_id",
        "failure_status",
        "failure_evidence",
        "required_fix",
    ] {
        if !present(item.get(required)) {
            issues.push(issue(
                "inventory_shape_repair",
                item,
                required,
                &format!("remediation item is missing {required}"),
            ));
        }
    }
    if !present(item.get("focused_verification")) && !present(item.get("verification_requirements"))
    {
        issues.push(issue(
            "verification_requirements_discovery",
            item,
            "focused_verification",
            "remediation item is missing focused verification",
        ));
    }
    if item.get("target_files").is_none() {
        issues.push(issue(
            "target_file_discovery",
            item,
            "target_files",
            "remediation item must include target_files",
        ));
    }
    for target in raw_strings(item, &["target_files"]) {
        if let Some(problem) = target_file_issue_message(contract, &target) {
            issues.push(issue(
                "target_file_discovery",
                item,
                "target_files",
                &format!("{problem}: {target}"),
            ));
        }
    }
    if item.get("artifact_requirements").is_none() {
        issues.push(issue(
            "artifact_requirements_discovery",
            item,
            "artifact_requirements",
            "remediation item is missing artifact requirements",
        ));
    }
    issues
}

/// JS `normalizeRemediationInventory`.
pub(super) fn normalize_remediation_inventory(
    contract: &LifecycleContract<'_>,
    value: &Value,
) -> Value {
    let raw_items = inventory_source_items(value);
    let items: Vec<Value> = raw_items
        .iter()
        .map(|item| contract.normalize_item(item))
        .collect();
    let mut issues = inventory_source_issues(value);
    for item in &items {
        issues.extend(remediation_item_issues(contract, item));
    }
    let mut object = value.as_object().cloned().unwrap_or_default();
    object.insert("items".to_string(), Value::Array(items));
    object.insert("unresolved_issues".to_string(), Value::Array(issues));
    Value::Object(object)
}

/// JS `generatedContractInventorySourceItems` — with `items` present that
/// array wins; the deep root-walking fallback lives in the Rust contract twin
/// (`collect_generated_inventory_items` via normalize_inventory), so reuse the
/// normalized item list for item-less shapes.
fn inventory_source_items(value: &Value) -> Vec<Value> {
    if value.get("items").is_some_and(Value::is_array) {
        return array(value.get("items"));
    }
    super::workflow_live_generated_contract::lifecycle_inventory_source_items(value)
}

/// JS `normalizeRemediationInventoryForSources`.
pub(super) fn normalize_remediation_inventory_for_sources(
    contract: &LifecycleContract<'_>,
    value: &Value,
    source_items: &[Value],
    fallback_items: &[Value],
    source_call_id: &str,
) -> Value {
    let normalized = normalize_remediation_inventory(contract, value);
    let items: Vec<Value> = array(normalized.get("items"))
        .into_iter()
        .map(|item| {
            match remediation_source_for_item(
                contract,
                &item,
                source_items,
                fallback_items,
                source_call_id,
            ) {
                Some(source) => contract.normalize_item(&remediation_item_with_source_ownership(
                    contract, &item, &source,
                )),
                None => item,
            }
        })
        .collect();
    let mut issues = inventory_source_issues(&normalized);
    for item in &items {
        issues.extend(remediation_item_issues(contract, item));
    }
    let mut object = normalized.as_object().cloned().unwrap_or_default();
    object.insert("items".to_string(), Value::Array(items));
    object.insert("unresolved_issues".to_string(), Value::Array(issues));
    Value::Object(object)
}

/// JS `remediationSourceForItem` + `remediationSourceById` + `remediationSourceByTask`.
fn remediation_source_for_item(
    contract: &LifecycleContract<'_>,
    item: &Value,
    source_items: &[Value],
    fallback_items: &[Value],
    source_call_id: &str,
) -> Option<Value> {
    let sources: Vec<Value> = source_items
        .iter()
        .map(|source| contract.normalize_item(source))
        .collect();
    let fallbacks: Vec<Value> = fallback_items
        .iter()
        .map(|source| contract.normalize_item(source))
        .collect();
    remediation_source_by_id(item, &sources, source_call_id)
        .or_else(|| remediation_source_by_task(contract, item, &sources))
        .or_else(|| remediation_source_by_id(item, &fallbacks, source_call_id))
        .or_else(|| remediation_source_by_task(contract, item, &fallbacks))
}

fn remediation_source_by_id(
    item: &Value,
    sources: &[Value],
    source_call_id: &str,
) -> Option<Value> {
    let raw_ids = raw_strings(
        item,
        &[
            "source_item_id",
            "sourceItemId",
            "failed_item_id",
            "failedItemId",
            "item_id",
            "id",
        ],
    );
    let prefix = format!("{}-", source_call_id.trim());
    let mut ids = BTreeSet::new();
    for id in raw_ids {
        if let Some(stripped) = id.strip_prefix(&prefix) {
            if !stripped.is_empty() {
                ids.insert(stripped.to_string());
            }
        }
        ids.insert(id);
    }
    sources
        .iter()
        .find(|source| {
            ["item_id", "id"].iter().any(|key| {
                source
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|id| ids.contains(id))
            })
        })
        .cloned()
}

fn remediation_source_by_task(
    contract: &LifecycleContract<'_>,
    item: &Value,
    sources: &[Value],
) -> Option<Value> {
    let ids = contract.canonical_ids_for(item);
    if ids.is_empty() {
        return None;
    }
    let matches: Vec<&Value> = sources
        .iter()
        .filter(|source| {
            contract
                .canonical_ids_for(source)
                .iter()
                .any(|id| ids.contains(id))
        })
        .collect();
    (matches.len() == 1).then(|| matches[0].clone())
}

/// JS `remediationItemWithSourceOwnership`.
fn remediation_item_with_source_ownership(
    contract: &LifecycleContract<'_>,
    item: &Value,
    source: &Value,
) -> Value {
    let mut merged = item.as_object().cloned().unwrap_or_default();
    let targets = raw_strings(source, &["target_files"]);
    if !targets.is_empty() {
        merged.insert("target_files".to_string(), serde_json::json!(targets));
    }
    if contract.canonical_ids_for(item).is_empty() {
        let source_ids = contract.canonical_ids_for(source);
        if !source_ids.is_empty() {
            merged.insert(
                "canonical_task_ids".to_string(),
                serde_json::json!(source_ids),
            );
        }
    }
    for fallback in [
        "dependency_ids",
        "artifact_requirements",
        "focused_verification",
        "acceptance_criteria",
    ] {
        if !present(merged.get(fallback).map(|v| &*v)) && present(source.get(fallback)) {
            merged.insert(fallback.to_string(), source.get(fallback).cloned().unwrap());
        }
    }
    if !present(merged.get("source_item_id").map(|v| &*v)) {
        if let Some(source_id) = source
            .get("item_id")
            .or_else(|| source.get("id"))
            .and_then(Value::as_str)
        {
            merged.insert(
                "source_item_id".to_string(),
                Value::String(source_id.to_string()),
            );
        }
    }
    Value::Object(merged)
}

/// JS `remediationInventoryReady`.
pub(super) fn remediation_inventory_ready(inventory: &Value) -> bool {
    !array(inventory.get("items")).is_empty()
        && array(inventory.get("unresolved_issues")).is_empty()
}

/// JS `remediationTaskIdSet`.
pub(super) fn remediation_task_id_set(
    contract: &LifecycleContract<'_>,
    items: &[Value],
) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for item in items {
        for id in contract.canonical_ids_for(item) {
            ids.insert(id);
        }
    }
    ids
}

/// JS `filterRemediationInventoryByTaskIds`.
pub(super) fn filter_remediation_inventory_by_task_ids(
    contract: &LifecycleContract<'_>,
    inventory: &Value,
    allowed: &BTreeSet<String>,
) -> Value {
    if allowed.is_empty() {
        return inventory.clone();
    }
    let items: Vec<Value> = array(inventory.get("items"))
        .into_iter()
        .filter(|item| {
            contract
                .canonical_ids_for(item)
                .iter()
                .any(|id| allowed.contains(id))
        })
        .collect();
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    object.insert("items".to_string(), Value::Array(items));
    Value::Object(object)
}

/// JS body_b.js `reviewRemediationItemIssues`.
fn review_remediation_item_issues(contract: &LifecycleContract<'_>, item: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    if !(item.get("item_id").is_some() || item.get("id").is_some()) {
        issues.push(issue(
            "review_remediation_shape_repair",
            item,
            "item_id",
            "review remediation item is missing item_id/id",
        ));
    }
    if contract.canonical_ids_for(item).is_empty() {
        issues.push(issue(
            "review_remediation_task_reconcile",
            item,
            "canonical_task_ids",
            "review remediation item must use canonical taskUniverse IDs, not synthetic issue IDs",
        ));
    }
    for dep in strings_of(item.get("dependency_ids")) {
        if !contract.canonical_universe().contains(&dep) {
            issues.push(issue(
                "review_remediation_task_reconcile",
                item,
                "dependency_ids",
                "review remediation dependency is not a canonical task ID",
            ));
        }
    }
    for required in [
        "source_item_id",
        "failure_status",
        "failure_evidence",
        "required_fix",
        "focused_verification",
    ] {
        if !present(item.get(required)) {
            issues.push(issue(
                "review_remediation_shape_repair",
                item,
                required,
                &format!("review remediation item is missing {required}"),
            ));
        }
    }
    if item.get("target_files").is_none() {
        issues.push(issue(
            "review_remediation_target_discovery",
            item,
            "target_files",
            "review remediation item must include target_files, using [] only for artifact-only remediation",
        ));
    }
    for target in raw_strings(item, &["target_files"]) {
        if let Some(problem) = target_file_issue_message(contract, &target) {
            issues.push(issue(
                "review_remediation_target_discovery",
                item,
                "target_files",
                &format!("{problem}: {target}"),
            ));
        }
    }
    if item.get("artifact_requirements").is_none() {
        issues.push(issue(
            "review_remediation_artifact_discovery",
            item,
            "artifact_requirements",
            "review remediation item is missing artifact requirements",
        ));
    }
    if raw_strings(item, &["target_files"]).is_empty()
        && !present(item.get("artifact_requirements"))
    {
        issues.push(issue(
            "review_remediation_artifact_discovery",
            item,
            "artifact_requirements",
            "artifact-only remediation needs concrete artifact requirements",
        ));
    }
    issues
}

/// JS body_b.js `normalizeReviewRemediationInventory`.
pub(super) fn normalize_review_remediation_inventory(
    contract: &LifecycleContract<'_>,
    value: &Value,
) -> Value {
    let raw_items = inventory_source_items(value);
    let items: Vec<Value> = raw_items
        .iter()
        .map(|item| contract.normalize_item(item))
        .collect();
    let mut issues = inventory_source_issues(value);
    for item in &items {
        issues.extend(review_remediation_item_issues(contract, item));
    }
    let mut object = value.as_object().cloned().unwrap_or_default();
    object.insert("items".to_string(), Value::Array(items));
    object.insert("unresolved_issues".to_string(), Value::Array(issues));
    Value::Object(object)
}

/// JS body_b.js `reviewNeedsRemediation`.
pub(super) fn review_needs_remediation(review: &Value) -> bool {
    if matches!(
        review.get("status").and_then(Value::as_str),
        Some("accepted") | Some("noop")
    ) {
        return false;
    }
    if review.is_null() {
        return false;
    }
    !array(review.get("items")).is_empty()
        || !array(review.get("residual_gaps")).is_empty()
        || !array(review.get("evidence")).is_empty()
        || present(review.get("summary"))
}

/// JS body_b.js `reviewRemediationInput`.
pub(super) fn review_remediation_input(review: &Value) -> Value {
    let items = array(review.get("items"));
    if !items.is_empty() {
        Value::Array(items)
    } else {
        review.clone()
    }
}
