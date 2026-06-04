//! BehaviourManifestVersion helpers.
//!
//! Load latest version of a kind, traverse the parent chain, and apply
//! a diff to produce new version content.

use anyhow::Result;
use cozo::DbInstance;

use crate::errors::LearningError;
use crate::models::*;
use crate::store;

/// Load the latest (current) version of a manifest kind.
pub fn load_current(
    db: &DbInstance,
    manifest_kind: &BehaviourManifestKind,
) -> Result<Option<BehaviourManifestVersion>> {
    store::get_latest_manifest_version(db, manifest_kind.as_str())
}

/// Traverse the parent chain from a given version back to the root.
/// Returns versions in order: [current, parent, grandparent, ...].
pub fn traverse_parent_chain(
    db: &DbInstance,
    start_version_id: &str,
) -> Result<Vec<BehaviourManifestVersion>> {
    let mut chain = Vec::new();
    let mut current_id = Some(start_version_id.to_string());

    while let Some(vid) = current_id {
        let version = store::get_manifest_version(db, &vid)?.ok_or_else(|| {
            LearningError::ManifestVersionNotFound {
                version_id: vid.clone(),
            }
        })?;
        current_id = version.parent_version_id.clone();
        chain.push(version);
    }

    Ok(chain)
}

/// Verify that applying `diff` (RFC 6902 JSON Patch) to `current_content`
/// produces `new_content`. Returns `new_content` on match; returns an error
/// if the diff is invalid, the patch fails to apply, or the result differs
/// from `new_content`.
///
/// Acts as a consistency check for manifest evolution: the caller computes
/// the new content via some external mechanism, records a JSON Patch describing
/// what changed, and this function verifies the diff actually produces the
/// recorded new content. This catches accidental drift between the recorded
/// diff and the recorded content during evolution audits.
///
/// An empty or whitespace-only `diff` is treated as an empty patch (`[]`,
/// no operations). In that case `current_content` must already equal
/// `new_content` for the consistency check to pass.
pub fn apply_diff(
    current_content: &serde_json::Value,
    diff: &str,
    new_content: serde_json::Value,
) -> Result<serde_json::Value, LearningError> {
    let trimmed = diff.trim();
    let patch_json = if trimmed.is_empty() { "[]" } else { trimmed };

    let patch: json_patch::Patch =
        serde_json::from_str(patch_json).map_err(|e| LearningError::Validation {
            message: format!("invalid JSON Patch (RFC 6902): {e}"),
        })?;

    let mut computed = current_content.clone();
    json_patch::patch(&mut computed, &patch).map_err(|e| LearningError::Validation {
        message: format!("failed to apply diff: {e}"),
    })?;

    if computed != new_content {
        return Err(LearningError::Validation {
            message: "diff inconsistent with recorded new content".into(),
        });
    }

    Ok(new_content)
}

/// Check whether a given version is the current head for its kind.
pub fn is_current_head(db: &DbInstance, version: &BehaviourManifestVersion) -> Result<bool> {
    let latest = store::get_latest_manifest_version(db, version.manifest_kind.as_str())?;
    match latest {
        Some(latest) => Ok(latest.version_id == version.version_id),
        None => Ok(false),
    }
}

#[cfg(test)]
mod apply_diff_tests {
    use super::apply_diff;
    use serde_json::json;

    #[test]
    fn empty_diff_with_identical_content_succeeds() {
        let current = json!({"k": 1});
        let new = json!({"k": 1});
        let result = apply_diff(&current, "", new.clone()).unwrap();
        assert_eq!(result, new);
    }

    #[test]
    fn whitespace_diff_treated_as_empty_patch() {
        let current = json!({"a": "b"});
        let result = apply_diff(&current, "   \n  ", current.clone()).unwrap();
        assert_eq!(result, current);
    }

    #[test]
    fn empty_diff_with_different_content_errors_as_inconsistent() {
        let current = json!({"k": 1});
        let new = json!({"k": 2});
        let err = apply_diff(&current, "", new).unwrap_err();
        assert!(err.to_string().contains("inconsistent"), "got error: {err}");
    }

    #[test]
    fn replace_patch_matching_new_content_succeeds() {
        let current = json!({"version": 1, "label": "alpha"});
        let new = json!({"version": 2, "label": "alpha"});
        let diff = r#"[{"op": "replace", "path": "/version", "value": 2}]"#;
        let result = apply_diff(&current, diff, new.clone()).unwrap();
        assert_eq!(result, new);
    }

    #[test]
    fn replace_patch_mismatched_new_content_errors_as_inconsistent() {
        let current = json!({"version": 1});
        let new = json!({"version": 3}); // says 3 but diff sets 2
        let diff = r#"[{"op": "replace", "path": "/version", "value": 2}]"#;
        let err = apply_diff(&current, diff, new).unwrap_err();
        assert!(err.to_string().contains("inconsistent"), "got error: {err}");
    }

    #[test]
    fn add_patch_succeeds() {
        let current = json!({});
        let new = json!({"new_key": "val"});
        let diff = r#"[{"op": "add", "path": "/new_key", "value": "val"}]"#;
        let result = apply_diff(&current, diff, new.clone()).unwrap();
        assert_eq!(result, new);
    }

    #[test]
    fn remove_patch_succeeds() {
        let current = json!({"a": 1, "b": 2});
        let new = json!({"a": 1});
        let diff = r#"[{"op": "remove", "path": "/b"}]"#;
        let result = apply_diff(&current, diff, new.clone()).unwrap();
        assert_eq!(result, new);
    }

    #[test]
    fn malformed_json_errors_as_invalid_patch() {
        let current = json!({});
        let new = json!({});
        let err = apply_diff(&current, "not valid json", new).unwrap_err();
        assert!(
            err.to_string().contains("invalid JSON Patch"),
            "got error: {err}"
        );
    }

    #[test]
    fn patch_op_failure_errors_as_apply_failed() {
        // Trying to remove a path that doesn't exist in current_content.
        let current = json!({"a": 1});
        let new = json!({"a": 1});
        let diff = r#"[{"op": "remove", "path": "/nonexistent"}]"#;
        let err = apply_diff(&current, diff, new).unwrap_err();
        assert!(
            err.to_string().contains("failed to apply diff"),
            "got error: {err}"
        );
    }

    #[test]
    fn multi_op_patch_succeeds() {
        let current = json!({"a": 1, "b": 2});
        let new = json!({"a": 99, "c": 3});
        let diff = r#"[
            {"op": "replace", "path": "/a", "value": 99},
            {"op": "remove", "path": "/b"},
            {"op": "add", "path": "/c", "value": 3}
        ]"#;
        let result = apply_diff(&current, diff, new.clone()).unwrap();
        assert_eq!(result, new);
    }
}
