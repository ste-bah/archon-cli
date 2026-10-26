//! Shared recognition of an intact skeleton made stale by an acceptance publish.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::{
    AcceptancePin, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest,
};
use archon_workflow::task_skeleton::{TaskSkeleton, TaskSkeletonLock};

/// The acceptance pin has advanced, so this intact predecessor skeleton is
/// waiting for the normal skeleton phase to freeze it against the new pin.
/// Malformed or digest-mismatched artifacts are errors, never treated as absent.
pub(crate) fn is_pending(task_root: &Path, pin: &AcceptancePin) -> Result<bool> {
    if pin.skeleton_digest.is_some() || pin.skeleton_gate.is_some() {
        return Ok(false);
    }
    let lock_path = task_root.join(TASK_SKELETON_LOCK_FILE);
    let skeleton_path = task_root.join(TASK_SKELETON_FILE);
    if !lock_path.exists() || !skeleton_path.exists() {
        return Ok(false);
    }
    let lock: TaskSkeletonLock = serde_json::from_slice(
        &std::fs::read(&lock_path)
            .with_context(|| format!("reading frozen skeleton lock {}", lock_path.display()))?,
    )
    .with_context(|| format!("parsing frozen skeleton lock {}", lock_path.display()))?;
    let bytes = std::fs::read(&skeleton_path)
        .with_context(|| format!("reading frozen skeleton {}", skeleton_path.display()))?;
    let actual = content_digest(&bytes);
    if lock.algorithm != "blake3" || lock.digest != actual {
        return Err(anyhow!(
            "frozen task skeleton under {} does not verify: lock expected {}, actual {}; restore the frozen version",
            task_root.display(),
            lock.digest,
            actual
        ));
    }
    let skeleton: TaskSkeleton = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing frozen skeleton {}", skeleton_path.display()))?;
    Ok(lock.acceptance_digest != pin.acceptance_digest
        && skeleton.acceptance_digest == lock.acceptance_digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::task_set_contract::{FreezeGateMode, FreezeGateStamp};

    fn pin(acceptance_digest: &str) -> AcceptancePin {
        AcceptancePin {
            task_root: "tasks/set".into(),
            acceptance_digest: acceptance_digest.into(),
            freeze_event_id: "acceptance-freeze".into(),
            acceptance_gate: stamp(),
            skeleton_digest: None,
            skeleton_gate: None,
            fidelity_waivers: Vec::new(),
            lineage: Vec::new(),
            lineage_recording: None,
            check_sources_digest: None,
        }
    }

    fn stamp() -> FreezeGateStamp {
        FreezeGateStamp {
            mode: FreezeGateMode::Observe,
            finding_count: 0,
            findings_digest: "findings".into(),
            binary_commit: "build".into(),
            evaluated_at: "now".into(),
        }
    }

    fn frozen(root: &Path, acceptance_digest: &str) {
        let skeleton = TaskSkeleton {
            schema_version: 1,
            acceptance_digest: acceptance_digest.into(),
            tasks: Vec::new(),
        };
        let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
        std::fs::write(root.join(TASK_SKELETON_FILE), &bytes).unwrap();
        let lock = TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: content_digest(&bytes),
            acceptance_digest: acceptance_digest.into(),
            gate: stamp(),
        };
        std::fs::write(
            root.join(TASK_SKELETON_LOCK_FILE),
            serde_json::to_vec_pretty(&lock).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn intact_prior_skeleton_is_pending_until_current_acceptance_freeze() {
        let root = tempfile::tempdir().unwrap();
        frozen(root.path(), "old");
        assert!(is_pending(root.path(), &pin("new")).unwrap());
        assert!(!is_pending(root.path(), &pin("old")).unwrap());
        let mut already_frozen = pin("new");
        already_frozen.skeleton_digest = Some("digest".into());
        assert!(!is_pending(root.path(), &already_frozen).unwrap());
    }

    #[test]
    fn a_skeleton_that_does_not_match_its_lock_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        frozen(root.path(), "old");
        std::fs::write(root.path().join(TASK_SKELETON_FILE), b"tampered").unwrap();
        let error = is_pending(root.path(), &pin("new")).unwrap_err();
        assert!(error.to_string().contains("does not verify"), "{error}");
    }
}
