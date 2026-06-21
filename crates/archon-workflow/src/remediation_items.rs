use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::reducers::{ReducerInput, ReducerOutput};
use crate::spec::{StageSpec, stage_declares_items_producer};

pub(crate) fn structured_items_output(
    stage: &StageSpec,
    inputs: &[ReducerInput],
) -> Option<ReducerOutput> {
    if !stage_declares_items_producer(stage) {
        return None;
    }
    let items = dedupe_items(
        inputs
            .iter()
            .filter_map(|input| items_from_text(&input.content))
            .flatten()
            .collect(),
    );
    let body = serde_json::to_string_pretty(&json!({ "items": items })).ok()?;
    Some(ReducerOutput {
        title: "Structured Items".to_string(),
        body,
        accepted_inputs: inputs.iter().filter(|input| input.accepted).count(),
        failed_inputs: inputs.iter().filter(|input| input.failed).count(),
        dissent: Vec::new(),
    })
}

pub(crate) fn items_from_text(body: &str) -> Option<Vec<Value>> {
    for candidate in candidate_documents(body) {
        if let Some(items) = parse_items_document(&candidate) {
            return Some(items);
        }
    }
    None
}

fn parse_items_document(body: &str) -> Option<Vec<Value>> {
    let value = serde_json::from_str::<Value>(body)
        .ok()
        .or_else(|| serde_yaml_ng::from_str::<Value>(body).ok())?;
    let mut items = Vec::new();
    collect_items(&value, &mut items).then_some(items)
}

fn collect_items(value: &Value, items: &mut Vec<Value>) -> bool {
    match value {
        Value::Object(map) => collect_object_items(map, items),
        Value::Array(values) => {
            let mut found = false;
            for value in values {
                found = collect_items(value, items) || found;
            }
            found
        }
        _ => false,
    }
}

fn collect_object_items(map: &Map<String, Value>, items: &mut Vec<Value>) -> bool {
    let mut found = false;
    if let Some(values) = map.get("items").and_then(Value::as_array) {
        items.extend(values.iter().cloned());
        found = true;
    }
    if let Some(values) = map.get("findings").and_then(Value::as_array) {
        items.extend(values.iter().filter_map(finding_to_item));
        found = true;
    }
    for value in map.values() {
        found = collect_items(value, items) || found;
    }
    found
}

fn finding_to_item(value: &Value) -> Option<Value> {
    let map = value.as_object()?;
    let target_files = string_array_from_keys(
        map,
        &["target_files", "files", "affected_files", "paths", "path"],
    );
    if target_files.is_empty() {
        return None;
    }
    let mut item = Map::new();
    copy_first_string(
        map,
        &["finding_id", "id", "finding"],
        &mut item,
        "finding_id",
    );
    copy_first_string(
        map,
        &["related_task_id", "task_id", "task"],
        &mut item,
        "related_task_id",
    );
    copy_first_string(map, &["severity", "priority"], &mut item, "severity");
    copy_first_string(map, &["failure", "problem", "issue"], &mut item, "failure");
    copy_first_string(
        map,
        &["required_fix", "fix", "recommendation"],
        &mut item,
        "required_fix",
    );
    copy_string_array(
        map,
        &["required_tests", "tests", "verification"],
        &mut item,
        "required_tests",
    );
    item.insert(
        "target_files".to_string(),
        Value::Array(target_files.into_iter().map(Value::String).collect()),
    );
    if !item.contains_key("task") {
        item.insert("task".to_string(), task_from_item(&item));
    }
    Some(Value::Object(item))
}

fn task_from_item(item: &Map<String, Value>) -> Value {
    let fix = item
        .get("required_fix")
        .and_then(Value::as_str)
        .or_else(|| item.get("failure").and_then(Value::as_str))
        .unwrap_or("Apply the required remediation.");
    Value::String(fix.to_string())
}

fn string_array_from_keys(map: &Map<String, Value>, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(values) = string_array(map.get(*key)) {
            return values;
        }
    }
    Vec::new()
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    match value? {
        Value::String(value) if !value.trim().is_empty() => Some(vec![value.trim().to_string()]),
        Value::Array(values) => Some(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        _ => None,
    }
}

fn copy_first_string(
    source: &Map<String, Value>,
    keys: &[&str],
    target: &mut Map<String, Value>,
    output_key: &str,
) {
    if let Some(value) = keys
        .iter()
        .filter_map(|key| source.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
    {
        target.insert(output_key.to_string(), Value::String(value.to_string()));
    }
}

fn copy_string_array(
    source: &Map<String, Value>,
    keys: &[&str],
    target: &mut Map<String, Value>,
    output_key: &str,
) {
    let values = string_array_from_keys(source, keys);
    if !values.is_empty() {
        target.insert(
            output_key.to_string(),
            Value::Array(values.into_iter().map(Value::String).collect()),
        );
    }
}

fn dedupe_items(items: Vec<Value>) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter(|item| {
            let key = item
                .get("finding_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| serde_json::to_string(item).ok())
                .unwrap_or_else(|| item.to_string());
            seen.insert(key)
        })
        .collect()
}

fn candidate_documents(body: &str) -> Vec<String> {
    let mut docs = vec![body.trim().to_string()];
    docs.extend(fenced_blocks(body));
    docs.extend(balanced_json_objects(body));
    docs
}

fn fenced_blocks(body: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("```") {
        rest = &rest[start + 3..];
        if let Some(newline) = rest.find('\n') {
            rest = &rest[newline + 1..];
        }
        let Some(end) = rest.find("```") else {
            break;
        };
        blocks.push(rest[..end].trim().to_string());
        rest = &rest[end + 3..];
    }
    blocks
}

fn balanced_json_objects(body: &str) -> Vec<String> {
    let mut docs = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;
    for (idx, ch) in body.char_indices() {
        if in_string {
            escape = ch == '\\' && !escape;
            if ch == '"' && !escape {
                in_string = false;
            }
            if ch != '\\' {
                escape = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0
                    && let Some(start_idx) = start.take()
                {
                    docs.push(body[start_idx..=idx].trim().to_string());
                }
            }
            _ => {}
        }
    }
    docs
}
