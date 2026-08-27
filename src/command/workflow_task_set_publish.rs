//! Atomic all-or-nothing publication for prepared task-set freezes.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, AcceptancePin,
    TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
};
use archon_workflow::task_skeleton::TaskSkeletonLock;

pub(super) fn publish_skeleton_files(
    tasks_root: &Path,
    pin_path: &Path,
    skeleton_bytes: &[u8],
    lock: &TaskSkeletonLock,
    pin: &AcceptancePin,
) -> Result<()> {
    publish_files_atomically(
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

pub(super) fn publish_acceptance_files(
    tasks_root: &Path,
    project_root: &Path,
    contract_bytes: &[u8],
    lock: &AcceptanceLock,
    pin: &AcceptancePin,
) -> Result<()> {
    let pin_path = super::acceptance_pin_path(project_root, tasks_root);
    if let Some(parent) = pin_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    publish_files_atomically(
        &[
            (
                tasks_root.join(ACCEPTANCE_CONTRACT_FILE),
                contract_bytes.to_vec(),
            ),
            (
                tasks_root.join(ACCEPTANCE_LOCK_FILE),
                serde_json::to_vec_pretty(lock)?,
            ),
            (pin_path, serde_json::to_vec_pretty(pin)?),
        ],
        "workflow freeze-acceptance",
    )
}

pub(crate) fn publish_files_atomically(files: &[(PathBuf, Vec<u8>)], remedy: &str) -> Result<()> {
    for (target, _) in files {
        if target.exists() && !target.is_file() {
            return Err(anyhow!(
                "cannot publish freeze to {}: destination exists and is not a file; remove or relocate it, then re-run {remedy}",
                target.display()
            ));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let mut temps = Vec::new();
    for (target, bytes) in files {
        let temp = sibling_transaction_path(target, &suffix, "new");
        if let Err(error) = std::fs::write(&temp, bytes) {
            for (staged, _) in &temps {
                let _ = std::fs::remove_file(staged);
            }
            let _ = std::fs::remove_file(&temp);
            return Err(error).with_context(|| format!("writing {}", temp.display()));
        }
        temps.push((temp, target.clone()));
    }
    let mut backups = Vec::new();
    let mut published = Vec::new();
    let operation = (|| -> Result<()> {
        for (target, _) in files {
            if target.exists() {
                let backup = sibling_transaction_path(target, &suffix, "old");
                std::fs::rename(target, &backup).with_context(|| {
                    format!("backing up {} before publishing", target.display())
                })?;
                backups.push((target.clone(), backup));
            }
        }
        for (temp, target) in &temps {
            std::fs::rename(temp, target)
                .with_context(|| format!("publishing freeze to {}", target.display()))?;
            published.push(target.clone());
        }
        Ok(())
    })();
    if let Err(error) = operation {
        for target in published.iter().rev() {
            let _ = std::fs::remove_file(target);
        }
        let mut rollback_failures = Vec::new();
        for (target, backup) in backups.iter().rev() {
            if let Err(restore_error) = std::fs::rename(backup, target) {
                rollback_failures.push(format!(
                    "{} -> {}: {restore_error}",
                    backup.display(),
                    target.display()
                ));
            }
        }
        for (temp, _) in &temps {
            let _ = std::fs::remove_file(temp);
        }
        if rollback_failures.is_empty() {
            return Err(error);
        }
        return Err(error.context(format!(
            "freeze rollback also failed: {}",
            rollback_failures.join("; ")
        )));
    }
    for warning in cleanup_committed_backups(&backups, |path| std::fs::remove_file(path)) {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

pub(super) fn cleanup_committed_backups<F>(
    backups: &[(PathBuf, PathBuf)],
    mut remove: F,
) -> Vec<String>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let mut warnings = Vec::new();
    for (_, backup) in backups {
        if let Err(error) = remove(backup) {
            warnings.push(format!(
                "freeze is already committed, but transaction backup {} could not be removed: {error}; verify the live freeze, then remove the stale backup manually",
                backup.display()
            ));
        }
    }
    warnings
}

fn sibling_transaction_path(target: &Path, suffix: &str, role: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    target.with_file_name(format!(".{name}.{suffix}.{role}"))
}
