use std::collections::BTreeSet;

use serde_json::Value;

use super::{EvidenceBundle, EvidenceItem, collect_string, collect_strings};

pub(super) fn value_to_bundles(value: &Value, item_id: Option<String>) -> Vec<EvidenceBundle> {
    let mut bundles = Vec::new();
    collect_value_bundles(value, item_id, &mut bundles);
    bundles
}

fn collect_value_bundles(value: &Value, item_id: Option<String>, out: &mut Vec<EvidenceBundle>) {
    match value {
        Value::Object(map) => {
            if let Some(body) = map.get("body") {
                collect_body_bundles(body, item_id.clone(), out);
            }
            for key in ["evidence_bundles", "completed_items"] {
                if let Some(items) = map.get(key).and_then(Value::as_array) {
                    for item in items {
                        if let Some(bundle) = bundle_from_object(item, item_id.clone()) {
                            out.push(bundle);
                        }
                    }
                }
            }
            if let Some(bundle) = bundle_from_object(value, item_id) {
                out.push(bundle);
            }
        }
        Value::String(text) => {
            for doc in candidate_documents(text) {
                if let Ok(json) = serde_json::from_str::<Value>(doc) {
                    collect_value_bundles(&json, item_id.clone(), out);
                } else if let Ok(yaml) = serde_yaml_ng::from_str::<Value>(doc) {
                    if parsed_same_scalar(doc, &yaml) {
                        continue;
                    }
                    collect_value_bundles(&yaml, item_id.clone(), out);
                }
            }
        }
        _ => {}
    }
}

fn parsed_same_scalar(doc: &str, value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|parsed| parsed.trim() == doc.trim())
}

fn collect_body_bundles(value: &Value, item_id: Option<String>, out: &mut Vec<EvidenceBundle>) {
    match value {
        Value::String(text) => collect_value_bundles(&Value::String(text.clone()), item_id, out),
        other => collect_value_bundles(other, item_id, out),
    }
}

fn bundle_from_object(value: &Value, item_id: Option<String>) -> Option<EvidenceBundle> {
    let object = value.as_object()?;
    let mut work_unit_ids = BTreeSet::new();
    for key in [
        "work_unit_ids",
        "task_ids",
        "canonical_task_ids",
        "implemented_work_unit_ids",
        "implemented_task_ids",
        "implemented_canonical_task_ids",
        "completed_work_unit_ids",
        "completed_task_ids",
        "completed_canonical_task_ids",
    ] {
        collect_strings(object.get(key), &mut work_unit_ids);
    }
    for key in [
        "work_unit_id",
        "task_id",
        "canonical_task_id",
        "implemented_work_unit_id",
        "implemented_task_id",
        "implemented_canonical_task_id",
        "completed_work_unit_id",
        "completed_task_id",
        "completed_canonical_task_id",
    ] {
        collect_string(object.get(key), &mut work_unit_ids);
    }
    if work_unit_ids.is_empty() {
        return None;
    }
    let status = object
        .get("status")
        .or_else(|| object.get("verification_status"))
        .or_else(|| object.get("result"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|status| !status.is_empty())?
        .to_string();
    Some(EvidenceBundle {
        work_unit_ids: work_unit_ids.into_iter().collect(),
        status,
        evidence: evidence_items(value),
        residual_gaps: residual_gaps(object.get("residual_gaps")),
        source_item_id: item_id,
    })
}

fn evidence_items(value: &Value) -> Vec<EvidenceItem> {
    let mut out = Vec::new();
    let Some(object) = value.as_object() else {
        return out;
    };
    for key in [
        "evidence",
        "commands_run",
        "verification",
        "tests",
        "focused_tests",
        "required_tests",
        "test_results",
        "tests_run",
    ] {
        collect_evidence_array(key, object.get(key), &mut out);
    }
    for key in [
        "changed_files",
        "target_files",
        "files_changed",
        "source_files_changed",
        "source_files",
        "declared_target_files",
        "expected_target_files",
    ] {
        if let Some(values) = object.get(key).and_then(Value::as_array) {
            for value in values.iter().filter_map(Value::as_str) {
                out.push(path_evidence("file", value));
            }
        }
    }
    if let Some(values) = object.get("artifacts").and_then(Value::as_array) {
        for value in values {
            if let Some(path) = value.as_str() {
                out.push(path_evidence("artifact", path));
            } else if let Some(path) = value.get("path").and_then(Value::as_str) {
                out.push(path_evidence("artifact", path));
            }
        }
    }
    if let Some(summary) = object.get("summary").and_then(Value::as_str) {
        out.push(EvidenceItem {
            kind: "review".into(),
            role: None,
            path: None,
            artifact_path: None,
            command: None,
            exit_status: None,
            summary: Some(summary.to_string()),
        });
    }
    out
}

fn collect_evidence_array(key: &str, value: Option<&Value>, out: &mut Vec<EvidenceItem>) {
    let Some(values) = value.and_then(Value::as_array) else {
        return;
    };
    for item in values {
        match item {
            Value::Object(map) => out.push(EvidenceItem {
                kind: map
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or(if key == "commands_run" {
                        "command"
                    } else if key.contains("test") {
                        "test"
                    } else {
                        key
                    })
                    .to_string(),
                role: map.get("role").and_then(Value::as_str).map(str::to_string),
                path: map
                    .get("path")
                    .or_else(|| map.get("file"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                artifact_path: map
                    .get("artifact_path")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                command: map
                    .get("command")
                    .or_else(|| map.get("test"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                exit_status: map
                    .get("exit_status")
                    .or_else(|| map.get("status_code"))
                    .and_then(Value::as_i64)
                    .map(|v| v as i32),
                summary: map
                    .get("summary")
                    .or_else(|| map.get("result"))
                    .or_else(|| map.get("assertion"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }),
            Value::String(text) if !text.trim().is_empty() => out.push(EvidenceItem {
                kind: if key.contains("test") {
                    "test"
                } else {
                    "review"
                }
                .into(),
                role: None,
                path: None,
                artifact_path: None,
                command: None,
                exit_status: None,
                summary: Some(text.trim().to_string()),
            }),
            _ => {}
        }
    }
}

fn path_evidence(kind: &str, path: &str) -> EvidenceItem {
    EvidenceItem {
        kind: kind.into(),
        role: None,
        path: Some(path.to_string()),
        artifact_path: None,
        command: None,
        exit_status: None,
        summary: None,
    }
}

fn residual_gaps(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(text)) if !text.trim().is_empty() => vec![text.trim().to_string()],
        _ => Vec::new(),
    }
}

fn candidate_documents(body: &str) -> Vec<&str> {
    let mut docs = vec![body.trim()];
    let mut rest = body;
    while let Some(start) = rest.find("```") {
        rest = &rest[start + 3..];
        if let Some(newline) = rest.find('\n') {
            rest = &rest[newline + 1..];
        }
        let Some(end) = rest.find("```") else {
            break;
        };
        docs.push(rest[..end].trim());
        rest = &rest[end + 3..];
    }
    docs
}
