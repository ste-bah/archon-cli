use serde_json::Value;

use archon_workflow::generated_lifecycle_support as support;

pub(super) struct ManifestScope {
    item_id: String,
    task_ids: Vec<String>,
    value: Value,
}

pub(super) fn manifest_scopes(evidence: &[Value]) -> Vec<ManifestScope> {
    let mut scopes = Vec::new();
    for entry in evidence {
        let result = entry.get("result").unwrap_or(entry);
        let outcomes = result.get("data").unwrap_or(result);
        for outcome in support::outcomes_of(outcomes) {
            scopes.extend(scopes_for_outcome(&outcome));
        }
    }
    scopes
}

pub(super) fn stamp_manifest_scope(item: &mut Value, scopes: &[ManifestScope]) {
    let Some(scope) = matching_scope(item, scopes) else {
        return;
    };
    if let Some(object) = item.as_object_mut() {
        object.insert("write_coordination_scope".to_string(), scope.value.clone());
    }
}

fn scopes_for_outcome(outcome: &Value) -> Vec<ManifestScope> {
    let item_id = string_field(outcome, "item_id");
    let task_ids = support::strings_of(outcome.get("canonical_task_ids"));
    manifest_paths(outcome)
        .into_iter()
        .filter_map(|path| load_scope(&path, &item_id, &task_ids))
        .collect()
}

fn manifest_paths(outcome: &Value) -> Vec<String> {
    support::array(outcome.get("completion_evidence"))
        .into_iter()
        .flat_map(|entry| support::strings_of(entry.get("artifact_paths")))
        .filter(|path| is_write_coordination_manifest(path))
        .collect()
}

/// Whether a recorded artifact path points at a write-coordination manifest.
///
/// Matched against a separator-normalised copy. Recorded paths come from
/// whatever produced them, so on Windows they arrive as
/// `...\write-coordination\stages\write\manifests\branch.json` and the
/// `/`-delimited markers never matched. The manifest then went unrecognised
/// and verification items were built with no diff scope at all — a silent
/// weakening of the check, not a visible failure.
fn is_write_coordination_manifest(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized.contains("/write-coordination/") && normalized.contains("/manifests/")
}

fn load_scope(path: &str, fallback_id: &str, task_ids: &[String]) -> Option<ManifestScope> {
    let raw = std::fs::read_to_string(path).ok()?;
    let manifest: Value = serde_json::from_str(&raw).ok()?;
    if manifest.get("schema").and_then(Value::as_str) != Some("archon.workflow.patch_manifest.v1") {
        return None;
    }
    manifest.get("declared_target_files")?.as_array()?;
    let item_id = string_field(&manifest, "item_id");
    let value = serde_json::json!({
        "manifest_path": path,
        "item_id": item_id,
        "declared_target_files": manifest.get("declared_target_files"),
        "changed_files": manifest.get("changed_files"),
        "created_files": manifest.get("created_files"),
        "deleted_files": manifest.get("deleted_files"),
    });
    Some(ManifestScope {
        item_id: if item_id.is_empty() {
            fallback_id.to_string()
        } else {
            item_id
        },
        task_ids: task_ids.to_vec(),
        value,
    })
}

fn matching_scope<'a>(item: &Value, scopes: &'a [ManifestScope]) -> Option<&'a ManifestScope> {
    let source_id = string_field(item, "source_item_id");
    if let Some(scope) = scopes
        .iter()
        .rev()
        .find(|scope| ids_match(&source_id, &scope.item_id))
    {
        return Some(scope);
    }
    let task_ids = support::strings_of(item.get("canonical_task_ids"));
    scopes
        .iter()
        .rev()
        .find(|scope| task_ids.iter().any(|id| scope.task_ids.contains(id)))
}

fn ids_match(left: &str, right: &str) -> bool {
    !left.is_empty()
        && !right.is_empty()
        && (left == right
            || left.ends_with(&format!("-{right}"))
            || right.ends_with(&format!("-{left}")))
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
