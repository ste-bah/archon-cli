use std::path::{Component, Path, PathBuf};

use archon_workflow::v2::project_artifact_contract::artifact_requirement_paths_from_field;
use serde_json::{Map, Value};

pub(super) fn stamp_project_artifact_paths(
    object: &mut Map<String, Value>,
    project_root: &str,
) -> Vec<Value> {
    let paths = object
        .get("artifact_requirements")
        .map(artifact_requirement_paths_from_field)
        .unwrap_or_default();
    let resolved: Vec<(String, String)> = paths
        .into_iter()
        .filter_map(|raw| resolve_project_path(project_root, &raw))
        .collect();
    rewrite_artifact_requirements(object, &resolved);
    rewrite_focused_verification(object, project_root, &resolved);
    resolved
        .into_iter()
        .map(|(path, absolute_path)| serde_json::json!({ "path": path, "absolute_path": absolute_path }))
        .collect()
}

fn resolve_project_path(project_root: &str, raw: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() || has_parent_component(raw) {
        return None;
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return path
            .starts_with(project_root)
            .then(|| (raw.to_string(), raw.to_string()));
    }
    Some((raw.to_string(), join_project_path(project_root, raw)))
}

/// Join a project root and a repo-relative artifact path as a `/`-separated
/// string.
///
/// `PathBuf::join(..).display()` was used here, which emits native separators:
/// on Windows the same workflow produced `/project\.archon/data/report.json`.
/// These strings are embedded in prompt text and matched against artifact
/// references elsewhere in the spec, all of which use `/`, so a native
/// separator corrupts the reference rather than merely looking different.
fn join_project_path(project_root: &str, relative: &str) -> String {
    let root = project_root.trim_end_matches(['/', '\\']);
    let relative = relative.trim_start_matches(['/', '\\']);
    format!("{root}/{relative}")
}

fn has_parent_component(raw: &str) -> bool {
    Path::new(raw)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn rewrite_artifact_requirements(object: &mut Map<String, Value>, paths: &[(String, String)]) {
    let Some(requirements) = object.get_mut("artifact_requirements") else {
        return;
    };
    rewrite_value(requirements, paths, false);
}

fn rewrite_focused_verification(
    object: &mut Map<String, Value>,
    project_root: &str,
    paths: &[(String, String)],
) {
    let Some(verification) = object.get_mut("focused_verification") else {
        return;
    };
    rewrite_value(verification, paths, true);
    rewrite_dot_archon_paths(verification, project_root);
}

fn rewrite_value(value: &mut Value, paths: &[(String, String)], embedded: bool) {
    match value {
        Value::String(text) => rewrite_string(text, paths, embedded),
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| rewrite_value(value, paths, embedded)),
        Value::Object(object) => object
            .values_mut()
            .for_each(|value| rewrite_value(value, paths, embedded)),
        _ => {}
    }
}

fn rewrite_string(text: &mut String, paths: &[(String, String)], embedded: bool) {
    for (raw, absolute) in paths {
        if text == raw {
            *text = absolute.clone();
            return;
        }
        if embedded {
            *text = text.replace(raw, absolute);
        }
    }
}

fn rewrite_dot_archon_paths(value: &mut Value, project_root: &str) {
    match value {
        Value::String(text) => *text = expand_dot_archon_tokens(text, project_root),
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| rewrite_dot_archon_paths(value, project_root)),
        Value::Object(object) => object
            .values_mut()
            .for_each(|value| rewrite_dot_archon_paths(value, project_root)),
        _ => {}
    }
}

fn expand_dot_archon_tokens(text: &str, project_root: &str) -> String {
    text.split_inclusive(char::is_whitespace)
        .map(|token| expand_token(token, project_root))
        .collect()
}

fn expand_token(token: &str, project_root: &str) -> String {
    let Some(start) = token.find(".archon/") else {
        return token.to_string();
    };
    if token[..start]
        .chars()
        .any(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '-' | '_'))
    {
        return token.to_string();
    }
    let end = token[start..]
        .find(['\'', '"', ',', ';', ')', '}'])
        .map(|offset| start + offset)
        .unwrap_or(token.len());
    let raw = &token[start..end];
    let absolute = join_project_path(project_root, raw);
    format!("{}{}{}", &token[..start], absolute, &token[end..])
}

#[cfg(test)]
#[path = "workflow_live_v2_artifact_paths_tests.rs"]
mod tests;
