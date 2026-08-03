use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{LifecycleContract, array, strings_of};

/// Verification plan items are atomic scheduling units. A plan may name several
/// focused checks, but those checks run in one branch so retries cannot multiply
/// an item into successively larger `-check-N` fanouts.
pub fn split_focused_verification_items(
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
pub fn verification_items(contract: &LifecycleContract<'_>, inventory: &Value) -> Vec<Value> {
    let items: Vec<Value> = array(inventory.get("items"))
        .iter()
        .map(|item| contract.normalize_item(item))
        .collect();
    split_focused_verification_items(contract, &items)
}

pub fn retry_verification_items(contract: &LifecycleContract<'_>, inventory: &Value) -> Vec<Value> {
    let mut items = verification_items(contract, inventory)
        .into_iter()
        .map(canonicalize_retry_identity)
        .collect::<Vec<_>>();
    let mut base_counts = BTreeMap::new();
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
