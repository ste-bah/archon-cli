//! Merge same-task cargo verification items into one branch.
//!
//! A verification plan for one task was observed emitting one item per single
//! `cargo test` function — twelve branches each spinning up a full agent to
//! run one test, plus fmt and clippy branches, all serialized by the cargo
//! role. Every one of those agents re-reads the same context and re-warms the
//! same build to answer one line of the same question. One agent running the
//! twelve commands in sequence answers them all in a fraction of the
//! wall-clock and leaves the wave's parallel width to the non-cargo branches.
//!
//! Only cargo-command items merge, and only within one task: they already
//! serialize against each other (see `cargo_serial`), so batching them loses
//! no parallelism by construction. Non-cargo items are untouched. Matched on
//! command text alone; carries no task, provider or PRD knowledge.

use serde_json::{Map, Value};

use super::cargo_serial::item_has_cargo_commands;
use crate::generated_lifecycle_support as support;

/// Upper bound on commands per merged item, so one branch's evidence stays
/// reviewable and a single agent is never asked to prove dozens of checks.
const MAX_BATCHED_CHECKS: usize = 10;

/// Merge cargo-command verification items that verify the same task.
///
/// An item joins a batch only when it is unmistakably a pure command check:
/// it runs cargo, names the same `canonical_task_ids` and `source_item_id` as
/// its batch, and declares no write scope. Anything else — inspections,
/// artifact checks, items with declared targets — passes through untouched, in
/// original order.
pub fn batch_cargo_verification_items(items: Vec<Value>) -> Vec<Value> {
    let mut passthrough: Vec<Value> = Vec::new();
    let mut batches: Vec<(String, Vec<Value>)> = Vec::new();
    for item in items {
        if !batchable(&item) {
            passthrough.push(item);
            continue;
        }
        let key = batch_key(&item);
        match batches
            .iter_mut()
            .find(|(existing, batch)| *existing == key && batch.len() < MAX_BATCHED_CHECKS)
        {
            Some((_, batch)) => batch.push(item),
            None => batches.push((key, vec![item])),
        }
    }
    for (_, batch) in batches {
        passthrough.push(merge_batch(batch));
    }
    passthrough
}

fn batchable(item: &Value) -> bool {
    if !item_has_cargo_commands(item) {
        return false;
    }
    if support::strings_of(item.get("canonical_task_ids")).is_empty() {
        return false;
    }
    // A declared write scope means the item is more than a read-only command
    // check; leave it alone.
    item.get("write_coordination_scope")
        .and_then(|scope| scope.get("declared_target_files"))
        .and_then(Value::as_array)
        .is_none_or(|targets| targets.is_empty())
}

/// Batch by originating plan item only.
///
/// Keying on `canonical_task_ids` as well meant batching could only ever fire
/// on a plan that emitted several cargo items for one task. The shape observed
/// live is the opposite and at least as common: one cargo item per task, all
/// from the same source item, every key unique, nothing merged, nine agents
/// spun up to run nine commands strictly one after another.
///
/// Dropping the task from the key is sound for the same reason the module's
/// same-task rule was: cargo items serialize against each other regardless of
/// which task they belong to, so merging them across tasks costs no
/// parallelism either. `merge_batch` unions the task ids so the merged branch
/// still declares everything it answers for.
fn batch_key(item: &Value) -> String {
    item.get("source_item_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn merge_batch(batch: Vec<Value>) -> Value {
    if batch.len() == 1 {
        return batch.into_iter().next().expect("single-item batch");
    }
    let mut merged: Map<String, Value> = batch[0].as_object().cloned().unwrap_or_default();
    let ids: Vec<String> = batch
        .iter()
        .filter_map(|item| {
            item.get("item_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    merged.insert(
        "item_id".to_string(),
        Value::String(format!(
            "{}-cargo-batch",
            ids.first().map(String::as_str).unwrap_or("verification")
        )),
    );
    merged.insert("batched_from_item_ids".to_string(), serde_json::json!(ids));
    // A merged branch now spans tasks, so it must declare every task it
    // answers for — inheriting only the first item's ids would silently drop
    // the rest from coverage. Provenance is kept alongside it so a failure in
    // one command can still be attributed to the task that asked for it,
    // rather than condemning every task in the batch.
    let mut tasks: Vec<Value> = Vec::new();
    let mut provenance: Vec<Value> = Vec::new();
    for item in &batch {
        let item_tasks = support::array(item.get("canonical_task_ids"));
        for task in &item_tasks {
            if !tasks.contains(task) {
                tasks.push(task.clone());
            }
        }
        provenance.push(serde_json::json!({
            "item_id": item.get("item_id").or_else(|| item.get("id")),
            "canonical_task_ids": item_tasks,
            "focused_verification": support::array(item.get("focused_verification")),
        }));
    }
    if !tasks.is_empty() {
        merged.insert("canonical_task_ids".to_string(), Value::Array(tasks));
    }
    merged.insert(
        "batched_item_provenance".to_string(),
        Value::Array(provenance),
    );
    for field in ["focused_verification", "expected_evidence"] {
        let values: Vec<Value> = batch
            .iter()
            .flat_map(|item| support::array(item.get(field)))
            .collect();
        merged.insert(field.to_string(), Value::Array(values));
    }
    for field in [
        "artifact_requirements",
        "required_tools",
        "source_residual_gap_ids",
    ] {
        let mut union: Vec<Value> = Vec::new();
        for value in batch
            .iter()
            .flat_map(|item| support::array(item.get(field)))
        {
            if !union.contains(&value) {
                union.push(value);
            }
        }
        if !union.is_empty() {
            merged.insert(field.to_string(), Value::Array(union));
        }
    }
    Value::Object(merged)
}

#[cfg(test)]
#[path = "verify_batching_tests.rs"]
mod tests;
