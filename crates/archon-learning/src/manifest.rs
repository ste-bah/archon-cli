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

/// Compute a new version by applying a diff to the current version's content.
///
/// For now this is a simple replacement strategy: the diff describes what
/// changed, and the caller provides the new content directly. In a future
/// version this could parse unified diffs or JSON patches.
pub fn apply_diff(
    _current_content: &serde_json::Value,
    _diff: &str,
    new_content: serde_json::Value,
) -> serde_json::Value {
    new_content
}

/// Check whether a given version is the current head for its kind.
pub fn is_current_head(db: &DbInstance, version: &BehaviourManifestVersion) -> Result<bool> {
    let latest = store::get_latest_manifest_version(db, version.manifest_kind.as_str())?;
    match latest {
        Some(latest) => Ok(latest.version_id == version.version_id),
        None => Ok(false),
    }
}
