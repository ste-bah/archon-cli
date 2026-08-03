use super::*;

use archon_workflow::v2::project_artifact_contract::{
    ArtifactRequirementSplit, split_artifact_requirement_values,
};

pub(super) fn copy_artifact_requirement_aliases(
    value: &serde_json::Value,
    object: &mut serde_json::Map<String, serde_json::Value>,
) {
    let explicit = ARTIFACT_ALIASES
        .iter()
        .any(|key| value.get(*key).is_some());
    object.remove("artifact_requirements");
    let split = split_artifact_requirement_values(raw_values_from_aliases(
        value,
        ARTIFACT_ALIASES,
    ));
    append_artifact_split(object, split, explicit);
}

pub(super) fn copy_nested_artifact_requirement_aliases(
    value: &serde_json::Value,
    object: &mut serde_json::Map<String, serde_json::Value>,
) {
    let Some(required_evidence) = value
        .get("required_evidence")
        .or_else(|| value.get("requiredEvidence"))
        .or_else(|| value.get("expected_completion_evidence"))
        .or_else(|| value.get("expectedCompletionEvidence"))
    else {
        return;
    };
    let split = split_artifact_requirement_values(raw_values_from_aliases(
        required_evidence,
        NESTED_ARTIFACT_ALIASES,
    ));
    append_artifact_split(object, split, false);
}

pub(super) fn copy_nested_object_artifact_aliases(
    value: &serde_json::Value,
    object_aliases: &[&str],
    aliases: &[&str],
    object: &mut serde_json::Map<String, serde_json::Value>,
) {
    let values = object_aliases
        .iter()
        .filter_map(|key| value.get(*key))
        .flat_map(|nested| raw_values_from_aliases(nested, aliases))
        .collect::<Vec<_>>();
    append_artifact_requirement_values(object, values, false);
}

pub(super) fn append_artifact_requirement_values(
    object: &mut serde_json::Map<String, serde_json::Value>,
    values: Vec<serde_json::Value>,
    explicit: bool,
) {
    let split = split_artifact_requirement_values(values);
    append_artifact_split(object, split, explicit);
}

fn append_artifact_split(
    object: &mut serde_json::Map<String, serde_json::Value>,
    split: ArtifactRequirementSplit,
    explicit: bool,
) {
    if !split.paths.is_empty() {
        append_alias_values(object, "artifact_requirements", split.paths);
    } else if explicit {
        object.insert(
            "artifact_requirements".to_string(),
            serde_json::Value::Array(Vec::new()),
        );
    }
    if !split.evidence.is_empty() {
        append_alias_values(object, "expected_evidence", split.evidence);
    }
    if !split.invalid.is_empty() {
        append_alias_values(object, "artifact_requirement_issues", split.invalid);
    }
}

const ARTIFACT_ALIASES: &[&str] = &[
    "artifact_requirements",
    "artifactRequirements",
    "artifacts",
    "required_artifacts",
    "requiredArtifacts",
    "expected_artifacts",
    "expectedArtifacts",
    "artifact_checks",
    "artifactChecks",
    "project_artifact_requirements",
    "projectArtifactRequirements",
];

const NESTED_ARTIFACT_ALIASES: &[&str] = &[
    "artifact_paths",
    "artifactPaths",
    "artifacts",
    "expected_artifacts",
    "expectedArtifacts",
    "artifact_checks",
    "artifactChecks",
];
