use serde_json::Value;

use crate::generated_lifecycle_support as support;

pub struct ManifestScope {
    item_id: String,
    task_ids: Vec<String>,
    value: Value,
}

pub fn manifest_scopes(evidence: &[Value]) -> Vec<ManifestScope> {
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

pub fn stamp_manifest_scope(item: &mut Value, scopes: &[ManifestScope]) {
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
    // Prefer a scope that speaks ONLY for this item's tasks.
    //
    // Matching on any overlap hands a single-task verification the write scope
    // of a branch that also covered other tasks, and every file those other
    // tasks legitimately wrote then reads as an out-of-scope write. Observed
    // live: one task's focused verification failed on
    // `data_store/validation.rs`, a file TDL-050 neither declares nor appends
    // to — it belonged to a sibling task whose remediation ran in the same wave.
    // The deliverable checks passed every round; only the scope check failed,
    // and no retry could change it because the write and the declaration were
    // both correct and simply belonged to different tasks.
    if let Some(scope) = scopes.iter().rev().find(|scope| {
        !scope.task_ids.is_empty()
            && scope.task_ids.iter().all(|id| task_ids.contains(id))
            && task_ids.iter().any(|id| scope.task_ids.contains(id))
    }) {
        return Some(scope);
    }
    // No exclusive scope exists, so overlap is the best available attribution.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(item_id: &str, tasks: &[&str], observed: &[&str], declared: &[&str]) -> ManifestScope {
        ManifestScope {
            item_id: item_id.to_string(),
            task_ids: tasks.iter().map(|t| t.to_string()).collect(),
            value: serde_json::json!({
                "declared_target_files": declared,
                "changed_files": observed,
            }),
        }
    }

    fn item(tasks: &[&str]) -> Value {
        serde_json::json!({ "canonical_task_ids": tasks })
    }

    fn stamped(item: &Value) -> Vec<String> {
        item.get("write_coordination_scope")
            .and_then(|s| s.get("changed_files"))
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The live failure: TDL-050's verification took a multi-task scope and
    /// inherited `validation.rs`, which TASK-TDL-020 wrote and owns.
    #[test]
    fn a_single_task_item_does_not_inherit_another_tasks_writes() {
        let scopes = vec![
            scope(
                "impl-050",
                &["TASK-TDL-050"],
                &["providers/openbb_polygon.rs"],
                &["providers/openbb_polygon.rs"],
            ),
            scope(
                "wave-completion",
                &["TASK-TDL-020", "TASK-TDL-050"],
                &["data_store/validation.rs", "providers/openbb_polygon.rs"],
                &["data_store/validation.rs"],
            ),
        ];
        let mut it = item(&["TASK-TDL-050"]);
        stamp_manifest_scope(&mut it, &scopes);
        assert_eq!(
            stamped(&it),
            vec!["providers/openbb_polygon.rs".to_string()]
        );
    }

    /// With no exclusive scope, overlap is still the best attribution there is.
    #[test]
    fn overlap_is_used_when_no_exclusive_scope_exists() {
        let scopes = vec![scope(
            "wave-completion",
            &["TASK-TDL-020", "TASK-TDL-050"],
            &["data_store/validation.rs"],
            &["data_store/validation.rs"],
        )];
        let mut it = item(&["TASK-TDL-050"]);
        stamp_manifest_scope(&mut it, &scopes);
        assert_eq!(stamped(&it), vec!["data_store/validation.rs".to_string()]);
    }

    /// An item covering both tasks legitimately takes the shared scope.
    #[test]
    fn a_multi_task_item_takes_the_shared_scope() {
        let scopes = vec![scope(
            "wave-completion",
            &["TASK-TDL-020", "TASK-TDL-050"],
            &["data_store/validation.rs"],
            &["data_store/validation.rs"],
        )];
        let mut it = item(&["TASK-TDL-020", "TASK-TDL-050"]);
        stamp_manifest_scope(&mut it, &scopes);
        assert_eq!(stamped(&it), vec!["data_store/validation.rs".to_string()]);
    }

    /// source_item_id still wins outright when it matches.
    #[test]
    fn an_explicit_source_item_id_still_takes_precedence() {
        let scopes = vec![
            scope("impl-050", &["TASK-TDL-050"], &["a.rs"], &["a.rs"]),
            scope("impl-020", &["TASK-TDL-020"], &["b.rs"], &["b.rs"]),
        ];
        let mut it = serde_json::json!({
            "canonical_task_ids": ["TASK-TDL-050"],
            "source_item_id": "impl-020",
        });
        stamp_manifest_scope(&mut it, &scopes);
        assert_eq!(stamped(&it), vec!["b.rs".to_string()]);
    }
}
