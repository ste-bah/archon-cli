//! Letting a repair split one grouped inventory item into several.
//!
//! `merge_inventory_repair` folds a repair into the inventory by identity:
//! every item is indexed under `item:<id>` and one `task:<canonical id>` alias
//! per task it covers, and a repair item that hits an existing alias is merged
//! INTO that item. That is right for a repair which corrects an item, and
//! structurally unable to express the one repair the dependency-graph pass
//! exists to make — splitting an item that groups a task with its own
//! prerequisite into one item per task.
//!
//! Observed live, and it cost a run: the inventory carried
//! `noop-<a>-<b>-<c>-<d>` covering four tasks where the second depended on the
//! first, so the graph pass raised "inventory item groups '<b>' with its
//! prerequisite '<a>'". The repair answered correctly every time — one item
//! per task, six iterations running. Each split item's `task:<id>` alias
//! already pointed at the grouped item, so each was merged back into it; the
//! D74 identity guard then restored the grouped item's own
//! `canonical_task_ids` over the split's. The grouping survived untouched, the
//! issue count never fell, `adopt_inventory_repair` correctly discarded a
//! repair that resolved nothing, and after the budget the run blocked on the
//! original seven issues. Six correct answers, none applicable.
//!
//! A split is recognised by shape alone — no task, provider or PRD knowledge:
//! two or more repair items whose canonical task ids are each a proper subset
//! of one grouped item's, and whose union covers that item exactly. The
//! coverage requirement is what makes this safe against the concern the
//! tombstone rule protects: work is never shed, only redistributed, because
//! every task on the grouped item still has an item after the split.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{LifecycleContract, array, present, strings_of};

/// Primary keys (`item:<id>`) of grouped items that a repair supersedes by
/// splitting them into one item per task.
///
/// The caller drops these from the merge so the split items are inserted as
/// items in their own right rather than folded back into the group.
pub(super) fn grouped_items_superseded_by_splits(
    contract: &LifecycleContract<'_>,
    inventory_items: &[Value],
    repair_items: &[Value],
) -> BTreeSet<String> {
    let repairs: Vec<(String, BTreeSet<String>)> = repair_items
        .iter()
        .map(|item| contract.normalize_item(item))
        .filter_map(|item| {
            let id = item.get("item_id").and_then(Value::as_str)?.to_string();
            let ids: BTreeSet<String> = strings_of(item.get("canonical_task_ids"))
                .into_iter()
                .collect();
            (!ids.is_empty()).then_some((id, ids))
        })
        .collect();
    if repairs.len() < 2 {
        return BTreeSet::new();
    }

    let mut superseded = BTreeSet::new();
    for raw in inventory_items {
        let grouped = contract.normalize_item(raw);
        let Some(grouped_id) = grouped.get("item_id").and_then(Value::as_str) else {
            continue;
        };
        let grouped_ids: BTreeSet<String> = strings_of(grouped.get("canonical_task_ids"))
            .into_iter()
            .collect();
        // Only an item covering several tasks can be split.
        if grouped_ids.len() < 2 {
            continue;
        }
        // A repair item that IS this item (same id) is a correction, not a
        // split, however its task list reads.
        let parts: Vec<&BTreeSet<String>> = repairs
            .iter()
            .filter(|(id, ids)| {
                id != grouped_id
                    && !ids.is_empty()
                    && ids.is_subset(&grouped_ids)
                    && ids != &grouped_ids
            })
            .map(|(_, ids)| ids)
            .collect();
        if parts.len() < 2 {
            continue;
        }
        // Every task on the grouped item must survive on some split item.
        let covered: BTreeSet<&String> = parts.iter().flat_map(|ids| ids.iter()).collect();
        if grouped_ids.iter().all(|id| covered.contains(id)) {
            superseded.insert(format!("item:{grouped_id}"));
        }
    }
    superseded
}

/// Every alias a superseded grouped item would otherwise occupy, so the merge
/// can clear them before inserting the split items — a stale `task:<id>`
/// alias would recapture the very split that replaced it.
pub(super) fn superseded_aliases(
    contract: &LifecycleContract<'_>,
    inventory_items: &[Value],
    superseded: &BTreeSet<String>,
) -> BTreeMap<String, ()> {
    let mut aliases = BTreeMap::new();
    for raw in inventory_items {
        let item = contract.normalize_item(raw);
        let Some(id) = item.get("item_id").and_then(Value::as_str) else {
            continue;
        };
        if !superseded.contains(&format!("item:{id}")) {
            continue;
        }
        aliases.insert(format!("item:{id}"), ());
        for task in strings_of(item.get("canonical_task_ids")) {
            aliases.insert(format!("task:{task}"), ());
        }
    }
    aliases
}

/// JS `mergeInventoryRepair`: fold repaired items into the existing inventory
/// keyed by item_id then canonical task ids, replacing matches and appending
/// new items in first-seen order.
pub fn merge_inventory_repair(
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
    // A repair that splits a grouped item into one item per task cannot be
    // expressed by identity merging: each split item's `task:<id>` alias
    // already points at the group, so it folds back in and the grouping
    // survives. Recognise that shape up front and leave the grouped item out
    // of the merge entirely, so the splits land as items in their own right.
    // Only splits whose union covers the grouped item's tasks qualify, so no
    // scheduled work is shed.
    let inventory_items = array(inventory.get("items"));
    let superseded = grouped_items_superseded_by_splits(contract, &inventory_items, &repair_items);
    let stale_aliases = superseded_aliases(contract, &inventory_items, &superseded);
    for item in inventory_items {
        let item = contract.normalize_item(&item);
        if primary_key(&item).is_some_and(|key| superseded.contains(&key)) {
            continue;
        }
        put_item(item, &mut order, &mut merged);
    }
    // Clear aliases the superseded groups would have left behind, or a split
    // item would match the group it replaced.
    for alias in stale_aliases.keys() {
        merged.remove(alias);
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

#[cfg(test)]
#[path = "generated_lifecycle_split_tests.rs"]
mod tests;
