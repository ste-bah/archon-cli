//! Publication failures cannot leave stale ownership usable after a rebind.
use super::*;
use archon_workflow::task_set_contract::{ACCEPTANCE_LOCK_FILE, TASK_SKELETON_LOCK_FILE};

fn bounded(test: impl FnOnce() + Send + 'static) {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(test));
        send.send(result).unwrap();
    });
    receive
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("publication test must finish")
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[test]
fn skeleton_freeze_retention_failure_prevents_commit() {
    bounded(|| {
        let temp = tempfile::tempdir().unwrap();
        let (tasks, _) = seed_frozen_acceptance(&temp);
        let prd = temp.path().join("prds/PRD-X.md");
        std::fs::write(tasks.join(TASK_SKELETON_FILE), skeleton("TASK-X-010")).unwrap();
        freeze_skeleton(temp.path(), &tasks, &prd).unwrap();
        let pin_path = acceptance_pin_path(temp.path(), &tasks);
        let original = std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap();
        let pin_bytes = std::fs::read(&pin_path).unwrap();
        let lock_bytes = std::fs::read(tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap();
        let predecessor = pin_path
            .parent()
            .unwrap()
            .join("predecessors")
            .join(pin_path.file_name().unwrap());
        std::fs::remove_file(&predecessor).unwrap();
        std::fs::create_dir(&predecessor).unwrap();
        let mut next: serde_json::Value = serde_json::from_slice(&original).unwrap();
        let mut added = next["tasks"][0].clone();
        added["task_id"] = "TASK-X-020".into();
        added["file_name"] = "TASK-X-020-body.md".into();
        next["tasks"].as_array_mut().unwrap().push(added);
        let next = serde_json::to_vec(&next).unwrap();
        let mut pin: AcceptancePin = serde_json::from_slice(&pin_bytes).unwrap();
        let mut lock: TaskSkeletonLock = serde_json::from_slice(&lock_bytes).unwrap();
        lock.digest = content_digest(&next);
        pin.skeleton_digest = Some(lock.digest.clone());
        let result = publish_skeleton_files(&tasks, &pin_path, &next, &lock, &pin);
        assert!(
            result.is_err(),
            "retention failure must fail publication before commit"
        );
        let error = result.unwrap_err();
        assert!(
            error.downcast_ref::<std::io::Error>().is_some(),
            "typed retention I/O: {error:#}"
        );
        assert_eq!(std::fs::read(&pin_path).unwrap(), pin_bytes);
        assert_eq!(
            std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap(),
            original
        );
        assert_eq!(
            std::fs::read(tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap(),
            lock_bytes
        );
    });
}

#[test]
fn skeleton_freeze_failed_retention_then_acceptance_rebind_refuses_stale_replay() {
    bounded(|| {
        let temp = tempfile::tempdir().unwrap();
        let (tasks, _) = seed_frozen_acceptance(&temp);
        let prd = temp.path().join("prds/PRD-X.md");
        std::fs::write(tasks.join(TASK_SKELETON_FILE), skeleton("TASK-X-010")).unwrap();
        freeze_skeleton(temp.path(), &tasks, &prd).unwrap();
        let pin_path = acceptance_pin_path(temp.path(), &tasks);
        let predecessor = pin_path
            .parent()
            .unwrap()
            .join("predecessors")
            .join(pin_path.file_name().unwrap());
        let old_anchor = std::fs::read(&predecessor).unwrap();
        let old = std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap();
        let old_lock = std::fs::read(tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap();
        let cache = temp
            .path()
            .join(".archon/freeze-cache/skeletons")
            .join(content_digest(&old));
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(cache, &old).unwrap();
        let mut next: serde_json::Value = serde_json::from_slice(&old).unwrap();
        let mut added = next["tasks"][0].clone();
        added["task_id"] = "TASK-X-020".into();
        added["file_name"] = "TASK-X-020-body.md".into();
        next["tasks"].as_array_mut().unwrap().push(added);
        std::fs::write(tasks.join(TASK_SKELETON_FILE), next.to_string()).unwrap();
        freeze_skeleton(temp.path(), &tasks, &prd).unwrap();
        let newer = std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap();
        let current_pin = std::fs::read(&pin_path).unwrap();
        // Model an interrupted older retention write: its stale anchor remains.
        std::fs::write(&predecessor, &old_anchor).unwrap();
        std::fs::remove_file(&predecessor).unwrap();
        std::fs::create_dir(&predecessor).unwrap();
        let contract = std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
        let lock: AcceptanceLock =
            serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap())
                .unwrap();
        let mut pin: AcceptancePin = serde_json::from_slice(&current_pin).unwrap();
        pin.skeleton_digest = None;
        pin.skeleton_gate = None;
        let publish = || {
            super::super::super::publish::publish_acceptance_files(
                &tasks,
                temp.path(),
                &contract,
                &lock,
                &pin,
            )
        };
        let failed = publish();
        let unchanged_pin = std::fs::read(&pin_path).unwrap() == current_pin;
        std::fs::remove_dir(&predecessor).unwrap();
        std::fs::write(&predecessor, old_anchor).unwrap();
        publish().unwrap();
        std::fs::write(tasks.join(TASK_SKELETON_LOCK_FILE), old_lock).unwrap();
        let replay = prepare_skeleton_freeze_from_candidate(
            temp.path(),
            &tasks,
            &prd,
            archon_core::config::GateMode::Observe,
            old,
        );
        assert!(
            failed.is_err(),
            "failed outgoing retention must refuse acceptance rebind"
        );
        assert!(
            unchanged_pin,
            "failed rebind must keep the outgoing current pin"
        );
        assert!(
            replay.is_err(),
            "a stale lock/cache pair cannot erase the newer subject"
        );
        let error = replay.unwrap_err();
        assert!(
            !CandidateRejected::caused(&error),
            "stale authority is corruption: {error:#}"
        );
        assert!(format!("{error:#}").contains("host pin"), "{error:#}");
        assert_eq!(
            std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap(),
            newer
        );
    });
}
