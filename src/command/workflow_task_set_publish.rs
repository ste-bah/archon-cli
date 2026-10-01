//! Atomic all-or-nothing publication for prepared task-set freezes.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, AcceptancePin,
    TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest,
};
use archon_workflow::task_skeleton::TaskSkeletonLock;

/// An exclusive advisory lock on one task set's frozen chain, keyed by its
/// pin. Every host publisher of that chain — the whole-set acceptance freeze,
/// the skeleton freeze and the per-check repair — holds it while it writes, so
/// two of them never interleave. The OS releases it when the process ends, so
/// a crash never leaves a stale lock behind.
pub(crate) struct ChainLock {
    _file: std::fs::File,
}

impl ChainLock {
    pub(crate) fn acquire(pin_path: &Path) -> Result<Self> {
        let path = pin_path.with_extension("chain.lock");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening chain lock {}", path.display()))?;
        file.try_lock().map_err(|error| {
            anyhow!(
                "another freeze or repair of this task set holds {} ({error}); wait for it to finish",
                path.display()
            )
        })?;
        Ok(Self { _file: file })
    }
}

/// The bytes a target must still hold when the transaction replaces it:
/// `Some(Some(digest))`, `Some(None)` for absent; unlisted targets are not
/// checked.
pub(crate) type ExpectedPrior = [(PathBuf, Option<String>)];

pub(super) fn publish_skeleton_files(
    tasks_root: &Path,
    pin_path: &Path,
    skeleton_bytes: &[u8],
    lock: &TaskSkeletonLock,
    pin: &AcceptancePin,
) -> Result<()> {
    let _lock = ChainLock::acquire(pin_path)?;
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
    let _lock = ChainLock::acquire(&pin_path)?;
    // PLAN-11: the sources each check runs are pinned with the chain.
    let sidecar =
        super::check_sources::frozen_sidecar(project_root, tasks_root, contract_bytes, None)?;
    let mut pin = pin.clone();
    pin.check_sources_digest = Some(content_digest(&sidecar.1));
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
            sidecar,
            (pin_path, serde_json::to_vec_pretty(&pin)?),
        ],
        "workflow freeze-acceptance",
    )
}

pub(crate) fn publish_files_atomically(files: &[(PathBuf, Vec<u8>)], remedy: &str) -> Result<()> {
    let transaction = begin_publish(files, remedy, &[])?;
    for warning in transaction.commit() {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

/// A published but not yet committed freeze: every target already holds its
/// new bytes, and a hard-link backup of each prior version is kept until the
/// caller commits (drops the backups) or rolls back (restores them).
///
/// Crash safety: each target is replaced by one `rename` of a fully written
/// sibling, which POSIX makes atomic, so at every instant a target holds its
/// complete old or complete new bytes and is never missing. A crash before
/// commit leaves the `.<name>.<txn>.old` backups beside the targets.
#[must_use = "a publish transaction must be committed or rolled back"]
pub(crate) struct PublishTransaction {
    /// (target, backup of its prior bytes or None when it did not exist,
    /// digest of the bytes this transaction wrote there).
    replaced: Vec<Replaced>,
}

type Replaced = (PathBuf, Option<PathBuf>, String);

impl PublishTransaction {
    /// Keep the new bytes and drop the backups.
    pub(crate) fn commit(self) -> Vec<String> {
        let backups = self
            .replaced
            .iter()
            .filter_map(|(target, backup, _)| backup.clone().map(|backup| (target.clone(), backup)))
            .collect::<Vec<_>>();
        cleanup_committed_backups(&backups, |path| std::fs::remove_file(path))
    }

    /// Restore every prior version (removing targets that did not exist).
    pub(crate) fn roll_back(self) -> Result<()> {
        roll_back(&self.replaced)
    }
}

fn roll_back(replaced: &[Replaced]) -> Result<()> {
    let mut failures = Vec::new();
    for (target, backup, written) in replaced.iter().rev() {
        // Only undo our own write: a target someone replaced after this
        // transaction published is theirs now, and restoring over it would
        // be the very lost update the chain lock exists to prevent.
        let current = std::fs::read(target)
            .ok()
            .map(|bytes| content_digest(&bytes));
        if current.as_deref() != Some(written.as_str()) {
            failures.push(format!(
                "{}: changed after this publish, left as it is",
                target.display()
            ));
            continue;
        }
        let restored = match backup {
            Some(backup) => std::fs::rename(backup, target),
            None => std::fs::remove_file(target),
        };
        if let Err(error) = restored {
            failures.push(format!("{}: {error}", target.display()));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "freeze rollback failed for {}; the prior versions are the .old backups beside each target",
            failures.join("; ")
        ))
    }
}

pub(crate) fn begin_publish(
    files: &[(PathBuf, Vec<u8>)],
    remedy: &str,
    expected_prior: &ExpectedPrior,
) -> Result<PublishTransaction> {
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
        let written = (|| -> std::io::Result<()> {
            use std::io::Write;
            // Windows requires write access for FlushFileBuffers. Flush the
            // same handle that staged the bytes, before any target is replaced.
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()
        })();
        if let Err(error) = written {
            for (staged, _) in &temps {
                let _ = std::fs::remove_file(staged);
            }
            let _ = std::fs::remove_file(&temp);
            return Err(error).with_context(|| format!("writing {}", temp.display()));
        }
        temps.push((temp, target.clone()));
    }
    // Last look before the first rename: every target must still hold the
    // bytes the caller verified, or another writer got there first.
    let changed = expected_prior
        .iter()
        .filter(|(path, digest)| {
            std::fs::read(path).ok().map(|bytes| content_digest(&bytes)) != *digest
        })
        .map(|(path, _)| path.display().to_string())
        .collect::<Vec<_>>();
    if !changed.is_empty() {
        for (temp, _) in &temps {
            let _ = std::fs::remove_file(temp);
        }
        return Err(anyhow!(
            "the frozen chain changed since it was verified ({}); nothing was written — re-run against the current chain",
            changed.join(", ")
        ));
    }
    let mut replaced: Vec<Replaced> = Vec::new();
    let operation = (|| -> Result<()> {
        for ((temp, target), (_, bytes)) in temps.iter().zip(files) {
            let backup = if target.exists() {
                let backup = sibling_transaction_path(target, &suffix, "old");
                std::fs::hard_link(target, &backup).with_context(|| {
                    format!("backing up {} before publishing", target.display())
                })?;
                Some(backup)
            } else {
                None
            };
            if let Err(error) = std::fs::rename(temp, target) {
                if let Some(backup) = backup {
                    let _ = std::fs::remove_file(backup);
                }
                return Err(error)
                    .with_context(|| format!("publishing freeze to {}", target.display()));
            }
            replaced.push((target.clone(), backup, content_digest(bytes)));
        }
        Ok(())
    })();
    if let Err(error) = operation {
        for (temp, _) in &temps {
            let _ = std::fs::remove_file(temp);
        }
        return match roll_back(&replaced) {
            Ok(()) => Err(error),
            Err(rollback) => Err(error.context(rollback.to_string())),
        };
    }
    Ok(PublishTransaction { replaced })
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
