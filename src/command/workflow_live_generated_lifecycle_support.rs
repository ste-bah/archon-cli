//! Value-level helpers for the Rust decomposed-PRD lifecycle.
//!
//! Faithful ports of the scaffold's JS helpers (body_a/noop/remediation JS and
//! the contract preflight JS). Items are `serde_json::Value` objects exactly
//! as they were in the QuickJS realm; normalization delegates to the existing
//! Rust contract twin (`workflow_live_generated_contract`), which the contract
//! test suites already pin against the JS behavior.

use std::collections::BTreeSet;

use serde_json::Value;

use super::workflow_live_generated_contract::{
    GeneratedContractIssue, normalize_canonical_ids, normalize_generated_inventory_value_with_repo,
    normalize_generated_item_value_with_repo,
};
use super::workflow_live_task_universe::WorkflowV2TaskUniverse;

pub(super) fn array(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![other.clone()],
    }
}

/// JS `generatedContractPresent`: non-empty string/array/object, or any other
/// non-null value.
pub(super) fn present(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
        Some(_) => true,
    }
}

pub(super) fn strings_of(value: Option<&Value>) -> Vec<String> {
    array(value)
        .into_iter()
        .filter_map(|entry| match entry {
            Value::String(text) => {
                let trimmed = text.trim().to_string();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            _ => None,
        })
        .collect()
}

pub(super) fn raw_strings(value: &Value, keys: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for key in keys {
        for entry in array(value.get(*key)) {
            match entry {
                Value::String(text) => {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        out.push(trimmed);
                    }
                }
                Value::Object(map) => {
                    for inner in ["path", "command", "check", "id", "summary"] {
                        if let Some(Value::String(text)) = map.get(inner) {
                            let trimmed = text.trim().to_string();
                            if !trimmed.is_empty() {
                                out.push(trimmed);
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

pub(super) fn unique(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

pub(super) struct LifecycleContract<'a> {
    pub(super) task_universe: &'a WorkflowV2TaskUniverse,
    pub(super) target_repository_root: Option<&'a str>,
}

impl LifecycleContract<'_> {
    pub(super) fn canonical_universe(&self) -> BTreeSet<String> {
        self.task_universe
            .tasks
            .iter()
            .map(|task| task.canonical_task_id.clone())
            .collect()
    }

    pub(super) fn canonical_ids_for(&self, item: &Value) -> Vec<String> {
        normalize_canonical_ids(
            Some(self.task_universe),
            strings_of(item.get("canonical_task_ids")),
        )
        .canonical_ids
    }

    pub(super) fn normalize_canonical_id_fields(&self, value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut object = object
                    .iter()
                    .map(|(key, value)| (key.clone(), self.normalize_canonical_id_fields(value)))
                    .collect::<serde_json::Map<_, _>>();
                let raw_ids = ["canonical_task_ids", "canonicalTaskIds"]
                    .iter()
                    .find_map(|key| object.get(*key))
                    .map(|value| strings_of(Some(value)))
                    .unwrap_or_default();
                if !raw_ids.is_empty() {
                    let normalized = normalize_canonical_ids(Some(self.task_universe), raw_ids);
                    object.insert(
                        "canonical_task_ids".to_string(),
                        serde_json::json!(normalized.canonical_ids),
                    );
                    object.remove("canonicalTaskIds");
                    if !normalized.unresolved_ids.is_empty() {
                        object.insert(
                            "canonical_id_repair_issues".to_string(),
                            serde_json::json!([{
                                "kind": "task_universe_reconcile",
                                "field": "canonical_task_ids",
                                "unresolved_ids": normalized.unresolved_ids,
                            }]),
                        );
                    }
                }
                Value::Object(object)
            }
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|value| self.normalize_canonical_id_fields(value))
                    .collect(),
            ),
            _ => value.clone(),
        }
    }

    pub(super) fn dependency_ids_for(&self, item: &Value) -> Vec<String> {
        let universe = self.canonical_universe();
        strings_of(item.get("dependency_ids"))
            .into_iter()
            .filter(|id| universe.contains(id))
            .collect()
    }

    pub(super) fn invalid_dependency_ids_for(&self, item: &Value) -> Vec<String> {
        let universe = self.canonical_universe();
        strings_of(item.get("dependency_ids"))
            .into_iter()
            .filter(|id| !universe.contains(id))
            .collect()
    }

    pub(super) fn normalize_item(&self, item: &Value) -> Value {
        normalize_generated_item_value_with_repo(
            item,
            Some(self.task_universe),
            self.target_repository_root,
        )
        .value
    }

    /// JS `normalizeGeneratedInventory`: spread of the source object plus
    /// normalized `items` (support items filtered out) and `unresolved_issues`
    /// (source issues + item issues + graph issues).
    pub(super) fn normalize_inventory(&self, value: &Value) -> Value {
        let normalized = normalize_generated_inventory_value_with_repo(
            value,
            Some(self.task_universe),
            self.target_repository_root,
        );
        let mut object = value.as_object().cloned().unwrap_or_default();
        let mut issues: Vec<Value> = inventory_source_issues(value);
        issues.extend(
            normalized
                .issues
                .iter()
                .map(|issue| issue_to_value(issue.clone())),
        );
        object.insert("items".to_string(), Value::Array(normalized.items));
        object.insert("unresolved_issues".to_string(), Value::Array(issues));
        Value::Object(object)
    }
}

fn issue_to_value(issue: GeneratedContractIssue) -> Value {
    serde_json::to_value(&issue).unwrap_or_else(|_| {
        serde_json::json!({
            "kind": "inventory_shape_repair",
            "field": issue.field,
            "message": issue.message,
        })
    })
}

/// JS `generatedContractInventorySourceIssues`.
pub(super) fn inventory_source_issues(source: &Value) -> Vec<Value> {
    if source.get("items").is_some_and(Value::is_array) {
        return array(source.get("unresolved_issues"));
    }
    Vec::new()
}

/// JS `issuesOfKind`.
pub(super) fn issues_of_kind(inventory: &Value, kind: &str) -> Vec<Value> {
    array(inventory.get("unresolved_issues"))
        .into_iter()
        .filter(|issue| issue.get("kind").and_then(Value::as_str) == Some(kind))
        .collect()
}

/// JS `generatedContractInventoryHasIssues`.
pub(super) fn inventory_has_issues(inventory: &Value) -> bool {
    !array(inventory.get("unresolved_issues")).is_empty()
}

/// JS `generatedContractVerificationInventoryReady`.
pub(super) fn verification_inventory_ready(inventory: &Value) -> bool {
    !array(inventory.get("items")).is_empty() && !inventory_has_issues(inventory)
}

/// JS `mergeInventoryRepair`: fold repaired items into the existing inventory
/// keyed by item_id then canonical task ids, replacing matches and appending
/// new items in first-seen order.
pub(super) fn merge_inventory_repair(
    contract: &LifecycleContract<'_>,
    inventory: &Value,
    repair: &Value,
) -> Value {
    let data = repair.get("data");
    let data_items = data.and_then(|data| data.get("items"));
    let mut repair_items: Vec<Value> = Vec::new();
    let direct = repair
        .get("items")
        .filter(|value| present(Some(value)))
        .or_else(|| repair.get("inventory").and_then(|inner| inner.get("items")))
        .or(data_items);
    repair_items.extend(array(direct));
    for extra_key in [
        "repaired_items",
        "implementation_items",
        "verified_noop_items",
    ] {
        repair_items.extend(array(data.and_then(|data| data.get(extra_key))));
        repair_items.extend(array(data_items.and_then(|items| items.get(extra_key))));
    }
    if repair_items.is_empty() {
        return inventory.clone();
    }
    fn item_keys(item: &Value) -> Vec<String> {
        let mut keys = Vec::new();
        if let Some(id) = item.get("item_id").and_then(Value::as_str) {
            keys.push(format!("item:{id}"));
        }
        for id in strings_of(item.get("canonical_task_ids")) {
            keys.push(format!("task:{id}"));
        }
        keys
    }
    fn primary_key(item: &Value) -> Option<String> {
        item_keys(item).into_iter().next()
    }
    let mut order: Vec<String> = Vec::new();
    let mut merged: std::collections::BTreeMap<String, Value> = Default::default();
    fn put_item(
        item: Value,
        order: &mut Vec<String>,
        merged: &mut std::collections::BTreeMap<String, Value>,
    ) {
        let Some(key) = primary_key(&item) else {
            return;
        };
        if !merged.contains_key(&key) {
            order.push(key.clone());
        }
        for alias in item_keys(&item) {
            merged.insert(alias, item.clone());
        }
        merged.insert(key, item);
    }
    for item in array(inventory.get("items")) {
        put_item(contract.normalize_item(&item), &mut order, &mut merged);
    }
    for raw_repair_item in repair_items {
        let repair_item = contract.normalize_item(&raw_repair_item);
        let keys = item_keys(&repair_item);
        let matched = keys.iter().find(|key| merged.contains_key(*key)).cloned();
        let tombstone = ["remove", "tombstone", "deleted"]
            .iter()
            .any(|key| repair_item.get(*key) == Some(&Value::Bool(true)));
        if tombstone {
            // D74: no repair prompt grants item removal — a tombstone must not
            // shed scheduled work (and never becomes a new item). Genuine
            // completion is proven through the noop/verification lifecycle.
            continue;
        }
        if let Some(matched_key) = matched {
            let existing = merged.get(&matched_key).cloned().unwrap_or_default();
            let mut combined = existing.as_object().cloned().unwrap_or_default();
            for (key, value) in repair_item.as_object().cloned().unwrap_or_default() {
                combined.insert(key, value);
            }
            // D74: identity fields on a host-known item survive the merge; a
            // repair may add them when absent but never reassign them.
            for protected in ["canonical_task_ids", "source_item_id"] {
                if let Some(value) = existing.get(protected).filter(|value| present(Some(value))) {
                    combined.insert(protected.to_string(), value.clone());
                }
            }
            let combined = Value::Object(combined);
            if let Some(existing_primary) = primary_key(&existing) {
                merged.insert(existing_primary, combined.clone());
            }
            for alias in item_keys(&existing).into_iter().chain(item_keys(&combined)) {
                merged.insert(alias, combined.clone());
            }
            continue;
        }
        put_item(repair_item, &mut order, &mut merged);
    }
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    object.insert("unresolved_issues".to_string(), Value::Array(Vec::new()));
    object.insert(
        "items".to_string(),
        Value::Array(
            order
                .iter()
                .filter_map(|key| merged.get(key).cloned())
                .collect(),
        ),
    );
    Value::Object(object)
}

/// Verification plan items are atomic scheduling units. A plan may name several
/// focused checks, but those checks run in one branch so retries cannot multiply
/// an item into successively larger `-check-N` fanouts.
pub(super) fn split_focused_verification_items(
    contract: &LifecycleContract<'_>,
    items: &[Value],
) -> Vec<Value> {
    let mut scheduled = Vec::new();
    let mut seen_stems = BTreeSet::new();
    for raw in items {
        let item = contract.normalize_item(raw);
        let item_id = item
            .get("item_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("verification")
            .to_string();
        if seen_stems.insert(verification_schedule_identity(&item, &item_id)) {
            scheduled.push(item);
        }
    }
    scheduled
}

/// JS `generatedContractVerificationItems`.
pub(super) fn verification_items(
    contract: &LifecycleContract<'_>,
    inventory: &Value,
) -> Vec<Value> {
    let items: Vec<Value> = array(inventory.get("items"))
        .iter()
        .map(|item| contract.normalize_item(item))
        .collect();
    split_focused_verification_items(contract, &items)
}

pub(super) fn retry_verification_items(
    contract: &LifecycleContract<'_>,
    inventory: &Value,
) -> Vec<Value> {
    let mut items = verification_items(contract, inventory)
        .into_iter()
        .map(canonicalize_retry_identity)
        .collect::<Vec<_>>();
    let mut base_counts = std::collections::BTreeMap::new();
    for item in &items {
        if let Some(id) = item.get("item_id").and_then(Value::as_str) {
            *base_counts.entry(id.to_string()).or_insert(0usize) += 1;
        }
    }
    let mut used = BTreeSet::new();
    for item in &mut items {
        let base = item
            .get("item_id")
            .and_then(Value::as_str)
            .unwrap_or("retry-verification")
            .to_string();
        let mut id = base.clone();
        if base_counts.get(&base).copied().unwrap_or_default() > 1 {
            let discriminator = verification_semantic_discriminator(item)
                .unwrap_or_else(|| item.get("source_item_id").cloned().unwrap_or(Value::Null));
            id = format!(
                "{base}-variant-{:016x}",
                stable_value_fingerprint(&discriminator)
            );
        }
        let mut collision = 2usize;
        let candidate = id.clone();
        while !used.insert(id.clone()) {
            id = format!("{candidate}-variant-{collision}");
            collision += 1;
        }
        if let Some(object) = item.as_object_mut() {
            object.insert("item_id".to_string(), Value::String(id.clone()));
            object.insert("id".to_string(), Value::String(id));
        }
    }
    items
}

fn canonicalize_retry_identity(item: Value) -> Value {
    let mut object = item.as_object().cloned().unwrap_or_default();
    let raw_id = object
        .get("item_id")
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("verification")
        .to_string();
    let source_id = object
        .get("source_item_id")
        .and_then(Value::as_str)
        .unwrap_or(&raw_id)
        .to_string();
    let source_stem = strip_check_suffixes(&source_id);
    let id_stem = strip_check_suffixes(&raw_id)
        .strip_prefix("retry-")
        .unwrap_or_else(|| strip_check_suffixes(&raw_id));
    let stem = if source_stem.is_empty() {
        id_stem
    } else {
        source_stem.strip_prefix("retry-").unwrap_or(source_stem)
    };
    let retry_id = format!("retry-{stem}");
    object.insert("item_id".to_string(), Value::String(retry_id.clone()));
    object.insert("id".to_string(), Value::String(retry_id));
    object
        .entry("source_item_id".to_string())
        .or_insert(Value::String(raw_id));
    Value::Object(object)
}

fn verification_schedule_identity(item: &Value, item_id: &str) -> String {
    let stem = strip_check_suffixes(item_id);
    match verification_semantic_discriminator(item) {
        Some(discriminator) => format!("{stem}::{:016x}", stable_value_fingerprint(&discriminator)),
        None => stem.to_string(),
    }
}

fn verification_semantic_discriminator(item: &Value) -> Option<Value> {
    let mut gap_ids = strings_of(item.get("source_residual_gap_ids"));
    gap_ids.sort();
    gap_ids.dedup();
    if !gap_ids.is_empty() {
        return Some(serde_json::json!({ "source_residual_gap_ids": gap_ids }));
    }
    item.get("failed_predicate")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|predicate| !predicate.is_empty())
        .map(|predicate| serde_json::json!({ "failed_predicate": predicate }))
}

fn stable_value_fingerprint(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .unwrap_or_default()
        .into_iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        })
}

fn strip_check_suffixes(mut value: &str) -> &str {
    while let Some((stem, suffix)) = value.rsplit_once("-check-") {
        if !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            value = stem;
        } else {
            break;
        }
    }
    value
}

/// JS `generatedContractConstrainInventoryTasks`.
pub(super) fn constrain_inventory_tasks(
    contract: &LifecycleContract<'_>,
    inventory: &Value,
    allowed_task_ids: &[String],
) -> Value {
    let allowed: BTreeSet<&String> = allowed_task_ids.iter().collect();
    if allowed.is_empty() {
        return inventory.clone();
    }
    let mut items = Vec::new();
    let mut issues = array(inventory.get("unresolved_issues"));
    for raw in array(inventory.get("items")) {
        let item = contract.normalize_item(&raw);
        let task_ids = strings_of(item.get("canonical_task_ids"));
        let outside: Vec<&String> = task_ids.iter().filter(|id| !allowed.contains(id)).collect();
        if outside.is_empty() {
            items.push(item);
        } else {
            issues.push(serde_json::json!({
                "kind": "verification_requirements_discovery",
                "field": "canonical_task_ids",
                "message": "verification repair introduced out-of-scope canonical task IDs",
                "item_id": item.get("item_id").or_else(|| item.get("id")),
                "canonical_task_ids": task_ids,
            }));
        }
    }
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    object.insert("items".to_string(), Value::Array(items));
    object.insert("unresolved_issues".to_string(), Value::Array(issues));
    Value::Object(object)
}

/// JS `recordRepairAttempt`.
pub(super) fn record_repair_attempt(
    attempts: &mut Vec<Value>,
    call_id: &str,
    issue_kind: &str,
    issues: &[Value],
    result: &Value,
) {
    let canonical: Vec<String> = unique(
        issues
            .iter()
            .flat_map(|issue| strings_of(issue.get("canonical_task_ids")))
            .collect(),
    );
    attempts.push(serde_json::json!({
        "call_id": call_id,
        "issue_kind": issue_kind,
        "canonical_task_ids": canonical,
        "files_read": raw_strings(result, &["files_read", "filesRead"]),
        "commands_run": raw_strings(result, &["commands_run", "commandsRun", "commands"]),
        "artifact_paths_checked": raw_strings(result, &["artifact_paths", "artifactPaths", "artifacts"]),
        "redacted_env_keys_checked": raw_strings(result, &["env_keys_checked", "envKeysChecked", "redacted_env_keys_checked", "redactedEnvKeysChecked"]),
        "evidence_refs": raw_strings(result, &["evidence_refs", "evidenceRefs", "proof_references", "proofReferences", "proof_refs", "proofRefs"]),
        "routing": compact_routing_fields(result),
        "reason": result.get("summary").and_then(Value::as_str).unwrap_or("repair or investigation result recorded"),
    }));
}

fn compact_routing_fields(result: &Value) -> Value {
    let data = result
        .get("data")
        .or_else(|| result.get("result").and_then(|inner| inner.get("data")))
        .unwrap_or(result);
    let mut routing = serde_json::Map::new();
    for key in [
        "retry_items",
        "retryItems",
        "implementation_failures",
        "terminal_blockers",
        "terminalBlockers",
    ] {
        if let Some(value) = data.get(key).filter(|value| present(Some(value))) {
            routing.insert(
                key.to_string(),
                Value::Array(
                    array(Some(value))
                        .iter()
                        .map(compact_routing_item)
                        .collect(),
                ),
            );
        }
    }
    for key in [
        "failure_class",
        "failure_kind",
        "transport_attempts",
        "max_transport_attempts",
    ] {
        if let Some(value) = data.get(key) {
            routing.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(routing)
}

fn compact_routing_item(item: &Value) -> Value {
    serde_json::json!({
        "item_id": item.get("item_id").or_else(|| item.get("id")),
        "source_item_id": item.get("source_item_id"),
        "canonical_task_ids": item.get("canonical_task_ids"),
        "source_residual_gap_ids": item.get("source_residual_gap_ids"),
        "failure_class": item.get("failure_class"),
        "failure_kind": item.get("failure_kind"),
        "classification": item.get("classification"),
    })
}

#[path = "workflow_live_generated_lifecycle_outcomes.rs"]
mod outcomes;
pub(super) use outcomes::*;

#[cfg(test)]
#[path = "workflow_live_generated_lifecycle_support_tests.rs"]
mod tests;
