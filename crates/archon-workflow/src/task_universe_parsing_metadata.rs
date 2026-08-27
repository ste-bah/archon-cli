//! Typed YAML metadata fields whose shapes are richer than scalar lists.

use std::path::Path;

use crate::error::{WorkflowError, WorkflowResult};

pub(super) fn metadata_dependencies(
    path: &Path,
    metadata: &serde_json::Value,
) -> WorkflowResult<Vec<crate::task_skeleton::FrozenDependency>> {
    let values = match metadata.get("depends_on") {
        Some(serde_json::Value::Null) | None => Vec::new(),
        Some(serde_json::Value::String(value)) => vec![serde_json::Value::String(value.clone())],
        Some(serde_json::Value::Array(values)) => values.clone(),
        Some(other) => {
            return Err(WorkflowError::SpecInvalid(format!(
                "generated decomposed PRD workflow has unreadable depends_on in {}: expected a list of task ids or structured dependency objects, found {other}",
                path.display()
            )));
        }
    };
    let mut dependencies = Vec::new();
    for value in values {
        let mut dependency = match value {
            serde_json::Value::String(task_id) => crate::task_skeleton::FrozenDependency {
                task_id,
                ..Default::default()
            },
            object @ serde_json::Value::Object(_) => serde_json::from_value(object).map_err(|error| {
                WorkflowError::SpecInvalid(format!(
                    "generated decomposed PRD workflow has an unreadable structured depends_on entry in {}: {error}; declare task_id plus exactly one of consumes or ordering_only",
                    path.display()
                ))
            })?,
            other => {
                return Err(WorkflowError::SpecInvalid(format!(
                    "generated decomposed PRD workflow has unreadable depends_on entry {other} in {}; use a task id string or structured dependency object",
                    path.display()
                )));
            }
        };
        dependency.task_id = dependency.task_id.trim().to_string();
        if dependency.task_id.is_empty() {
            return Err(WorkflowError::SpecInvalid(format!(
                "generated decomposed PRD workflow has an empty depends_on task_id in {}; use a canonical TASK-<DOMAIN>-<NNN> id or an unambiguous legacy alias",
                path.display()
            )));
        }
        dependencies.push(dependency);
    }
    dependencies.sort();
    dependencies.dedup();
    Ok(dependencies)
}

pub(super) fn require_string_or_string_list_or_null(
    path: &Path,
    metadata: &serde_json::Value,
    field: &str,
) -> WorkflowResult<()> {
    let valid = match metadata.get(field) {
        Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(_)) => true,
        Some(serde_json::Value::Array(values)) => values.iter().all(serde_json::Value::is_string),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow field '{field}' in {} must be a string, list of strings, or null; rewrite mappings and non-string entries as strings or []",
            path.display()
        )))
    }
}

pub(super) fn require_string_or_string_list(
    path: &Path,
    metadata: &serde_json::Value,
    field: &str,
) -> WorkflowResult<()> {
    let valid = match metadata.get(field) {
        Some(serde_json::Value::String(_)) => true,
        Some(serde_json::Value::Array(values)) => values.iter().all(serde_json::Value::is_string),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow field '{field}' in {} must be a string or list of strings; rewrite mappings, nulls, and non-string entries as obligation-id strings",
            path.display()
        )))
    }
}

pub(super) fn metadata_string(metadata: &serde_json::Value, field: &str) -> Option<String> {
    metadata
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn metadata_strings(metadata: &serde_json::Value, field: &str) -> Vec<String> {
    match metadata.get(field) {
        Some(serde_json::Value::String(value)) => vec![value.clone()],
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}
