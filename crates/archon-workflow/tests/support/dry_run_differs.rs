//! The replay dry run's DIFFERS diagnostics and its approval rule.
use std::path::PathBuf;

use archon_workflow::*;
use serde_json::{Value, json};

/// What a DIFFERS call changed against its record: the option keys whose
/// values differ, and for a fan-out whether its items' prompts or evidence
/// did (the record keeps the items it dispatched).
pub fn why_differs(
    recorded: &WorkflowV2CallRecord,
    execution: &WorkflowV2CallExecution,
) -> Vec<String> {
    let old = serde_json::to_value(&recorded.call.options).unwrap();
    let new = serde_json::to_value(&execution.call.options).unwrap();
    let mut keys: Vec<String> = Vec::new();
    for (key, value) in old.as_object().into_iter().flatten() {
        if new.get(key) != Some(value) {
            keys.push(key.clone());
        }
    }
    for key in new.as_object().into_iter().flatten().map(|(k, _)| k) {
        if old.get(key).is_none() {
            keys.push(key.clone());
        }
    }
    let items: Vec<Value> = execution.input["source_data"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut item_fields: std::collections::BTreeSet<String> = Default::default();
    for (at, item) in recorded.dispatched_items.iter().enumerate() {
        let recorded_item = serde_json::to_value(item).unwrap();
        let now = items.get(at).cloned().unwrap_or(Value::Null);
        for field in ["task", "evidence", "canonical_task_ids"] {
            let before = recorded_item
                .get("input")
                .and_then(|i| i.get("item"))
                .and_then(|i| i.get(field))
                .or_else(|| recorded_item.get(field));
            if before.is_some() && before != now.get(field) {
                item_fields.insert(field.to_string());
            }
        }
    }
    println!("          changed options: {keys:?}; changed item fields: {item_fields:?}");
    if keys.iter().any(|k| k == "task") {
        let (a, b) = (
            old["task"].as_str().unwrap_or(""),
            new["task"].as_str().unwrap_or(""),
        );
        let at = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
        let show = |t: &str| {
            t.chars()
                .skip(at.saturating_sub(20))
                .take(140)
                .collect::<String>()
        };
        println!("          task was: …{}", show(a));
        println!("          task now: …{}", show(b));
    }
    keys
}

/// Every DIFFERS call that is not approved. A DIFFERS call passes only when it is an approved, explained change
/// (`ARCHON_DRY_RUN_APPROVED_DIFFERS`: `[{id, options, why}]`, exactly
/// the changed option keys), or a review reducer whose own options are
/// unchanged and one of whose source maps is an approved DIFFERS -- its
/// input moved only because that map's did. Anything else fails.
pub fn unexplained(
    differs: &[(String, Vec<String>, Vec<String>)],
    approved_file: Option<PathBuf>,
) -> Vec<String> {
    let approved: Vec<Value> = approved_file
        .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
        .unwrap_or_default();
    let approved_ids: Vec<&str> = differs
        .iter()
        .filter(|(id, keys, _)| {
            approved.iter().any(|entry| {
                entry["id"] == json!(id)
                    && entry["options"].as_array().is_some_and(|listed| {
                        listed
                            .iter()
                            .filter_map(Value::as_str)
                            .eq(keys.iter().map(String::as_str))
                    })
            })
        })
        .map(|(id, _, _)| id.as_str())
        .collect();
    let mut unexplained = Vec::new();
    for (id, keys, sources) in differs {
        if approved_ids.contains(&id.as_str()) {
            let why = approved
                .iter()
                .find(|e| e["id"] == json!(id))
                .map_or("", |e| e["why"].as_str().unwrap_or(""));
            println!("== DIFFERS {id} APPROVED: {why}");
        } else if keys.is_empty()
            && sources
                .iter()
                .any(|map| approved_ids.contains(&map.as_str()))
        {
            println!(
                "== DIFFERS {id} APPROVED: its options are unchanged; its source map {sources:?} is an approved change"
            );
        } else {
            unexplained.push(format!("{id} (changed options {keys:?})"));
        }
    }
    unexplained
}
