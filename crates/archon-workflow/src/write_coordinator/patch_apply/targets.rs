//! Which files a landing touches and what they hold: split from
//! `patch_apply.rs` to hold the 500-line ceiling.

use std::collections::BTreeMap;
use std::path::Path;

use super::super::ItemId;
use super::PatchManifest;

/// The first declared file this item intends to change whose canonical content
/// has moved since the patch was computed.
///
/// The comparison itself is `write_claim_gate::decide_write_claim`, so there is
/// ONE definition of "this baseline is stale" rather than a copy here and a
/// second one wherever the question gets asked next.
///
/// A target with no recorded pre-hash is skipped, not failed: that is a file
/// nothing captured a baseline for, and treating an unknown as a mismatch would
/// reject every legitimately new file.
pub(super) fn stale_target(
    canonical_root: &Path,
    m: &PatchManifest,
    pre_hashes_by_item: &BTreeMap<ItemId, BTreeMap<String, String>>,
) -> Option<String> {
    let expected = pre_hashes_by_item.get(&m.item_id)?;
    m.declared_target_files
        .iter()
        .filter(|t| m.changed_files.iter().any(|c| c == *t))
        .find(|t| {
            let Some(baseline) = expected.get(t.as_str()) else {
                return false;
            };
            let now = hash_file(&canonical_root.join(t)).unwrap_or_else(|| "absent".to_string());
            !crate::v2::write_claim_gate::decide_write_claim(t, Some(baseline), Some(&now))
                .should_proceed()
        })
        .cloned()
}

pub(crate) fn hash_file(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
}

/// Every path the manifest says landed, so `post_hashes` proves each one:
/// the declared targets (as before, a declared deletion hashing as
/// "deleted"), plus every changed or created file the diff carried that was
/// not declared — a file written inside a directory scope or under an
/// unreported-change grant. A deletion that was not declared stays out: the
/// path is absent, there is nothing to hash. Without the undeclared ones the
/// post-apply audit had no hash to match and reported the wave's own files
/// as an unexpected change (Issue-25).
pub(super) fn landed_paths(m: &PatchManifest) -> Vec<String> {
    let mut paths = m.declared_target_files.clone();
    for path in m.changed_files.iter().chain(&m.created_files) {
        if !paths.contains(path) && !m.deleted_files.contains(path) {
            paths.push(path.clone());
        }
    }
    paths
}

pub(super) fn hash_targets(canonical_root: &Path, targets: &[String]) -> BTreeMap<String, String> {
    targets
        .iter()
        .map(|t| {
            let h = hash_file(&canonical_root.join(t)).unwrap_or_else(|| "deleted".to_string());
            (t.clone(), h)
        })
        .collect()
}

/// Every repository file the patch changes, creates or deletes: what a
/// landing backs up before it applies, so a refusal after the apply puts
/// the tree back exactly (Batch K).
pub(super) fn touched_files(m: &PatchManifest) -> Vec<String> {
    let mut paths: Vec<String> = (m.changed_files.iter())
        .chain(&m.created_files)
        .chain(&m.deleted_files)
        .cloned()
        .collect();
    paths.sort();
    paths.dedup();
    paths
}
