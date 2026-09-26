//! Resume granularity for AC-WC-010, split from `patch_apply.rs` to hold the
//! 500-line ceiling.

use std::path::Path;

use super::{ApplyResumeStatus, ItemId, ManifestStatus, PatchManifest};

/// Resume granularity for AC-WC-010: load the persisted manifest status.
pub fn resume_status(item_id: &ItemId, run_root: &Path, stage_id: &str) -> ApplyResumeStatus {
    let path = run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id)
        .join("manifests")
        .join(format!("{item_id}.json"));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ApplyResumeStatus::NotPersisted;
    };
    let Ok(manifest) = serde_json::from_str::<PatchManifest>(&text) else {
        return ApplyResumeStatus::NotPersisted;
    };
    // Issue-113: copies placed where they are verified must still stand.
    if crate::v2::branch_cache::materialized::materialized_holds(run_root, &manifest).is_err() {
        return ApplyResumeStatus::NotPersisted;
    }
    match manifest.status {
        ManifestStatus::Applied => ApplyResumeStatus::Applied,
        ManifestStatus::IdempotentNoop => ApplyResumeStatus::IdempotentNoop,
        ManifestStatus::SkippedIgnored => ApplyResumeStatus::SkippedIgnored,
        ManifestStatus::Conflicted => ApplyResumeStatus::Conflicted,
        ManifestStatus::PendingApply => ApplyResumeStatus::PendingApply,
        ManifestStatus::Failed { reason } => ApplyResumeStatus::Failed(reason),
    }
}
