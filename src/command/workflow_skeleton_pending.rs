//! Shared recognition of a valid predecessor skeleton made stale by an acceptance publish.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::{
    AcceptancePin, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest, validate_gate_stamp,
};
use archon_workflow::task_skeleton::{TaskSkeleton, TaskSkeletonLock, validate_skeleton};

/// The acceptance pin has advanced, so this validated predecessor skeleton is
/// waiting for the normal skeleton phase to freeze it against the new pin.
/// Only the successor pin fields may be absent; malformed artifacts are errors.
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
    if lock.algorithm != "blake3" {
        return Err(anyhow!(
            "frozen task skeleton under {} uses unsupported lock algorithm {}; expected blake3",
            task_root.display(),
            lock.algorithm
        ));
    }
    validate_gate_stamp(&lock.gate, TASK_SKELETON_LOCK_FILE)
        .map_err(|error| anyhow!("frozen task skeleton gate is invalid: {error}"))?;
    let actual = content_digest(&bytes);
    if lock.digest != actual {
        return Err(anyhow!(
            "frozen task skeleton under {} does not verify: lock expected {}, actual {}; restore the frozen version",
            task_root.display(),
            lock.digest,
            actual
        ));
    }
    let skeleton: TaskSkeleton = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing frozen skeleton {}", skeleton_path.display()))?;
    if lock.acceptance_digest == pin.acceptance_digest {
        return Ok(false);
    }
    if skeleton.acceptance_digest != lock.acceptance_digest {
        return Err(anyhow!(
            "{} acceptance_digest is {}, but frozen skeleton {} binds {}; restore the matching frozen version",
            TASK_SKELETON_LOCK_FILE,
            lock.acceptance_digest,
            TASK_SKELETON_FILE,
            skeleton.acceptance_digest
        ));
    }
    let canonical_root = task_root.canonicalize()?;
    let pinned_root = Path::new(&pin.task_root)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(&pin.task_root).to_path_buf());
    if canonical_root != pinned_root {
        return Err(anyhow!(
            "acceptance pin binds task_root {}, actual {}; restore the frozen directory",
            pinned_root.display(),
            canonical_root.display()
        ));
    }
    validate_skeleton(&skeleton, &lock.acceptance_digest)
        .map_err(|error| anyhow!("frozen predecessor skeleton is invalid: {error}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::task_set_contract::{FreezeGateMode, FreezeGateStamp};
    use archon_workflow::task_skeleton::FrozenTask;

    fn pin(root: &Path, acceptance_digest: &str) -> AcceptancePin {
        AcceptancePin {
            task_root: root.canonicalize().unwrap().display().to_string(),
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
            findings_digest: archon_workflow::task_set_contract::empty_gate_findings_digest(),
            binary_commit: "build".into(),
            evaluated_at: "2026-08-26T18:30:00Z".into(),
        }
    }

    fn frozen(root: &Path, acceptance_digest: &str) {
        let skeleton = TaskSkeleton {
            schema_version: 1,
            acceptance_digest: acceptance_digest.into(),
            tasks: vec![FrozenTask {
                task_id: "TASK-X-010".into(),
                file_name: "TASK-X-010.md".into(),
                depends_on: Vec::new(),
                blocks: Vec::new(),
                implements: vec!["AC-X-001".into()],
                deliverable_contracts: Vec::new(),
            }],
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
        assert!(is_pending(root.path(), &pin(root.path(), "new")).unwrap());
        assert!(!is_pending(root.path(), &pin(root.path(), "old")).unwrap());
        let mut already_frozen = pin(root.path(), "new");
        already_frozen.skeleton_digest = Some("digest".into());
        assert!(!is_pending(root.path(), &already_frozen).unwrap());
    }

    #[test]
    fn a_skeleton_that_does_not_match_its_lock_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        frozen(root.path(), "old");
        std::fs::write(root.path().join(TASK_SKELETON_FILE), b"tampered").unwrap();
        let error = is_pending(root.path(), &pin(root.path(), "new")).unwrap_err();
        assert!(error.to_string().contains("does not verify"), "{error}");
    }

    #[test]
    fn invalid_gate_provenance_is_not_a_pending_skeleton() {
        let root = tempfile::tempdir().unwrap();
        frozen(root.path(), "old");
        let lock_path = root.path().join(TASK_SKELETON_LOCK_FILE);
        let mut lock: TaskSkeletonLock =
            serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
        lock.gate.findings_digest = "invalid".into();
        std::fs::write(lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();

        let error = is_pending(root.path(), &pin(root.path(), "new")).unwrap_err();
        assert!(error.to_string().contains("gate is invalid"), "{error}");
    }
}
