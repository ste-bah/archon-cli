//! Skeleton publication through the shared locked transaction path.
use super::*;

pub(crate) fn publish_skeleton_files(
    tasks_root: &Path,
    pin_path: &Path,
    skeleton_bytes: &[u8],
    lock: &TaskSkeletonLock,
    pin: &AcceptancePin,
) -> Result<()> {
    let _lock = ChainLock::acquire(pin_path, tasks_root)?;
    publish_files_atomically(
        pin_path,
        tasks_root,
        &[
            (tasks_root.join(TASK_SKELETON_FILE), skeleton_bytes.to_vec()),
            (
                tasks_root.join(TASK_SKELETON_LOCK_FILE),
                serde_json::to_vec_pretty(lock)?,
            ),
            (pin_path.to_path_buf(), serde_json::to_vec_pretty(pin)?),
        ],
        "workflow freeze-skeleton",
    )
}
