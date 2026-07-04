use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::project_artifacts::WorkflowV2ProjectArtifactContext;

pub(crate) fn project_artifact_prompt_section(
    input: &Value,
    context: &WorkflowV2ProjectArtifactContext,
) -> String {
    let Some(root) = context
        .project_root
        .as_deref()
        .filter(|root| !root.is_empty())
    else {
        return String::new();
    };
    let entries = resolved_project_artifact_entries(input, root);
    if entries.is_empty() {
        return String::new();
    }
    let mut section = String::from(
        "\n## Resolved Project Artifact Paths\n\
         Use these absolute paths for project artifacts. Do not resolve relative `.archon/...` \
         artifact paths against `repository_root`.\n",
    );
    for (raw, absolute) in entries {
        section.push_str(&format!("- {raw} => {absolute}\n"));
    }
    section
}

fn resolved_project_artifact_entries(input: &Value, root: &str) -> Vec<(String, String)> {
    let mut raw_paths = Vec::new();
    collect_artifact_paths(input, false, &mut raw_paths);
    let mut seen = BTreeSet::new();
    raw_paths
        .into_iter()
        .filter_map(|raw| resolved_project_artifact_path(root, &raw))
        .filter(|entry| seen.insert(entry.clone()))
        .collect()
}

fn collect_artifact_paths(value: &Value, in_artifact_list: bool, paths: &mut Vec<String>) {
    match value {
        Value::Array(items) => collect_artifact_array(items, in_artifact_list, paths),
        Value::Object(object) => {
            for (key, child) in object {
                let child_artifact_list = in_artifact_list || artifact_list_key(key);
                if child_artifact_list && path_key(key) {
                    push_string_path(child, paths);
                }
                collect_artifact_paths(child, child_artifact_list, paths);
            }
        }
        _ => {}
    }
}

fn collect_artifact_array(items: &[Value], in_artifact_list: bool, paths: &mut Vec<String>) {
    for item in items {
        if in_artifact_list {
            push_string_path(item, paths);
        }
        collect_artifact_paths(item, in_artifact_list, paths);
    }
}

fn push_string_path(value: &Value, paths: &mut Vec<String>) {
    if let Some(path) = value.as_str().filter(|path| !path.trim().is_empty()) {
        paths.push(path.to_string());
    }
}

fn artifact_list_key(key: &str) -> bool {
    matches!(
        key,
        "artifact_requirements"
            | "artifactRequirements"
            | "project_artifact_requirements"
            | "required_artifacts"
            | "requiredArtifacts"
            | "expected_artifacts"
            | "artifact_checks"
            | "artifacts"
            | "artifact_paths"
            | "artifactPaths"
    )
}

fn path_key(key: &str) -> bool {
    matches!(key, "path" | "artifact_path" | "artifactPath")
}

fn resolved_project_artifact_path(root: &str, raw: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() || path_has_parent_component(raw) {
        return None;
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return path
            .starts_with(root)
            .then(|| (raw.to_string(), raw.to_string()));
    }
    Some((
        raw.to_string(),
        PathBuf::from(root).join(path).display().to_string(),
    ))
}

fn path_has_parent_component(raw: &str) -> bool {
    Path::new(raw)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
}
