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

/// Keys that name the path an artifact declaration points at.
///
/// `absolute_path` belongs here because the inventory emits it. Without it,
/// `{"absolute_path": "...", "kind": "...", "must_exist": true}` matched no key,
/// found no evidence text either, and fell through to `invalid` — which becomes
/// `artifact_requirement_issues`, which
/// `generated_contract_validation` reports as "artifact declarations must be
/// concrete paths". The declaration WAS concrete; the reader did not know the
/// key.
///
/// Nothing could clear that. The repair prompt tells the reducer to fix
/// `artifact_requirements`, while the flag lives in a different field derived
/// from re-splitting the same values, so a correct repair was re-classified
/// invalid every round: six attempts in one run, three in another, five months
/// apart on two different agents, with the issue count never moving off seven.
fn explicit_path(object: &serde_json::Map<String, Value>) -> Option<&str> {
    [
        "path",
        "artifact_path",
        "artifactPath",
        "absolute_path",
        "absolutePath",
    ]
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

/// A path carrying template placeholders or glob syntax can never be checked
/// for literal existence; callers must exclude it from literal artifact
/// evidence and rely on pattern-based deliverable contracts instead.
pub(crate) fn artifact_path_is_templated(raw: &str) -> bool {
    artifact_text_has_pattern(raw.trim())
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

#[cfg(test)]
mod absolute_path_tests {
    use super::*;

    /// The live shape the inventory emits. Without `absolute_path` in the key
    /// list this fell through to `invalid`, became
    /// `artifact_requirement_issues`, and the validator reported the
    /// declaration as non-concrete — which no repair could clear.
    #[test]
    fn an_absolute_path_declaration_is_a_path_not_an_issue() {
        let split = split_artifact_requirement_values(vec![serde_json::json!({
            "absolute_path": "/Volumes/work/project-1/docs/trading/data-lake-gap-audit.md",
            "kind": "data_lake_gap_report",
            "must_exist": true,
        })]);
        assert_eq!(split.paths.len(), 1, "must be a concrete path");
        assert!(
            split.invalid.is_empty(),
            "must not be an issue: {:?}",
            split.invalid
        );
    }

    #[test]
    fn the_camel_case_spelling_is_accepted_too() {
        let split = split_artifact_requirement_values(vec![serde_json::json!({
            "absolutePath": "/tmp/project/reports/out.json",
        })]);
        assert_eq!(split.paths.len(), 1);
        assert!(split.invalid.is_empty());
    }

    /// An object naming no path and carrying no evidence text is still invalid.
    #[test]
    fn an_object_with_neither_path_nor_evidence_stays_invalid() {
        let split = split_artifact_requirement_values(vec![serde_json::json!({
            "kind": "something",
            "must_exist": true,
        })]);
        assert_eq!(split.invalid.len(), 1);
        assert!(split.paths.is_empty());
    }
}
