//! Recognize an intact skeleton awaiting its freeze after acceptance changed.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::{
    AcceptancePin, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest,
};
use archon_workflow::task_skeleton::{TaskSkeleton, TaskSkeletonLock};

/// A newer acceptance pin has no skeleton provenance until the skeleton phase
/// rebinds it. The previous skeleton is pending only if its lock still hashes
/// the bytes and those bytes name the lock's previous acceptance digest.
pub(super) fn intact_stale_skeleton(task_root: &Path, pin: &AcceptancePin) -> Result<bool> {
    if pin.skeleton_digest.is_some() || pin.skeleton_gate.is_some() {
        return Ok(false);
    }
    let lock_path = task_root.join(TASK_SKELETON_LOCK_FILE);
    let skeleton_path = task_root.join(TASK_SKELETON_FILE);
    let lock: TaskSkeletonLock = serde_json::from_slice(
        &std::fs::read(&lock_path)
            .with_context(|| format!("reading frozen skeleton lock {}", lock_path.display()))?,
    )
    .with_context(|| format!("parsing frozen skeleton lock {}", lock_path.display()))?;
    let skeleton_bytes = std::fs::read(&skeleton_path)
        .with_context(|| format!("reading frozen skeleton {}", skeleton_path.display()))?;
    let actual_digest = content_digest(&skeleton_bytes);
    if lock.algorithm != "blake3" || lock.digest != actual_digest {
        return Err(anyhow!(
            "frozen task skeleton under {} does not verify: lock expected {}, actual {}; restore the frozen version",
            task_root.display(),
            lock.digest,
            actual_digest
        ));
    }
    let skeleton: TaskSkeleton = serde_json::from_slice(&skeleton_bytes)
        .with_context(|| format!("parsing frozen skeleton {}", skeleton_path.display()))?;
    Ok(lock.acceptance_digest != pin.acceptance_digest
        && skeleton.acceptance_digest == lock.acceptance_digest)
}
