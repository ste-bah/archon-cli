use std::collections::BTreeSet;

use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactRequirementSplit {
    pub paths: Vec<Value>,
    pub evidence: Vec<Value>,
    pub invalid: Vec<Value>,
}

pub fn split_artifact_requirement_values(values: Vec<Value>) -> ArtifactRequirementSplit {
    let mut split = ArtifactRequirementSplit::default();
    for value in values {
        split_artifact_value(value, &mut split);
    }
    split.paths = dedupe_values(split.paths);
    split.evidence = dedupe_values(split.evidence);
    split.invalid = dedupe_values(split.invalid);
    split
}

pub fn artifact_requirement_paths(value: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    collect_artifact_paths(value, false, &mut paths);
    dedupe_strings(paths)
}

pub fn artifact_requirement_paths_from_field(value: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    collect_artifact_paths(value, true, &mut paths);
    dedupe_strings(paths)
}

fn split_artifact_value(value: Value, split: &mut ArtifactRequirementSplit) {
    match value {
        Value::Array(items) => {
            for item in items {
                split_artifact_value(item, split);
            }
        }
        Value::Object(object) => split_artifact_object(object, split),
        Value::String(text) if artifact_text_is_path(&text) => {
            split.paths.push(Value::String(text))
        }
        Value::String(text) if !text.trim().is_empty() => split.evidence.push(Value::String(text)),
        _ => {}
    }
}

fn split_artifact_object(
    object: serde_json::Map<String, Value>,
    split: &mut ArtifactRequirementSplit,
) {
    if let Some(path) = explicit_path(&object) {
        if artifact_text_is_path(path) {
            split.paths.push(Value::Object(object));
        } else {
            split.evidence.push(Value::String(path.to_string()));
        }
        return;
    }
    if let Some(evidence) = object_evidence(&object) {
        split.evidence.push(Value::String(evidence.to_string()));
        return;
    }
    split.invalid.push(Value::Object(object));
}

fn collect_artifact_paths(value: &Value, in_artifact_field: bool, paths: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_artifact_paths(item, in_artifact_field, paths);
            }
        }
        Value::Object(object) => collect_object_artifact_paths(object, in_artifact_field, paths),
        Value::String(text) if in_artifact_field && artifact_text_is_path(text) => {
            paths.push(text.trim().to_string())
        }
        _ => {}
    }
}

fn collect_object_artifact_paths(
    object: &serde_json::Map<String, Value>,
    in_artifact_field: bool,
    paths: &mut Vec<String>,
) {
    if in_artifact_field
        && let Some(path) = explicit_path(object).filter(|path| artifact_text_is_path(path))
    {
        paths.push(path.trim().to_string());
    }
    for (key, value) in object {
        collect_artifact_paths(value, artifact_list_key(key), paths);
    }
}

fn explicit_path(object: &serde_json::Map<String, Value>) -> Option<&str> {
    ["path", "artifact_path", "artifactPath"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

fn object_evidence(object: &serde_json::Map<String, Value>) -> Option<&str> {
    [
        "expected_evidence",
        "required_evidence",
        "summary",
        "description",
    ]
    .iter()
    .find_map(|key| object.get(*key).and_then(Value::as_str))
    .map(str::trim)
    .filter(|value| !value.is_empty())
}

fn artifact_list_key(key: &str) -> bool {
    matches!(
        key,
        "artifact_requirements"
            | "artifactRequirements"
            | "project_artifact_requirements"
            | "projectArtifactRequirements"
            | "required_artifacts"
            | "requiredArtifacts"
            | "expected_artifacts"
            | "expectedArtifacts"
            | "artifact_checks"
            | "artifactChecks"
            | "artifacts"
            | "artifact_paths"
            | "artifactPaths"
    )
}

fn artifact_text_is_path(raw: &str) -> bool {
    let text = raw.trim();
    if text.is_empty() || text.chars().any(|ch| ch == '\n' || ch == '\r') {
        return false;
    }
    if text.split_whitespace().count() > 1 {
        return false;
    }
    if artifact_text_has_pattern(text) {
        return false;
    }
    text.starts_with('/')
        || text.starts_with("./")
        || text.starts_with(".archon/")
        || text.starts_with("artifacts/")
        || text.contains('/')
        || text.contains('\\')
}

fn artifact_text_has_pattern(text: &str) -> bool {
    text.chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '<' | '>'))
}

fn dedupe_values(values: Vec<Value>) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.to_string()))
        .collect()
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}
