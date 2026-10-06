//! Crash-atomic all-or-nothing publication for prepared task-set freezes.
//!
//! The commit point of a publish is one atomic rename: the publish journal
//! (`journal` module) moving to `committed`. A kill at any instant leaves the
//! journal in the state that decides the outcome, and recovery
//! (`recover` module) — run by every publisher under the set's publish lock,
//! by the chain lock's acquisition, and at run launch and resume — rolls the
//! set back to the complete old version or forward to the complete new one.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, AcceptancePin,
    TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest,
};
use archon_workflow::task_skeleton::TaskSkeletonLock;

#[path = "workflow_task_set_publish_journal.rs"]
mod journal;
#[path = "workflow_task_set_publish_legacy.rs"]
mod legacy;
#[path = "workflow_task_set_publish_lock.rs"]
mod lock;
pub(crate) use lock::{ChainRead, UnsettledPublish, register_publish_settle};
#[path = "workflow_task_set_publish_verification.rs"]
mod verification;
pub(crate) fn valid_recovery_transaction(value: &str) -> bool {
    journal::is_transaction_id(value)
}
pub(crate) use legacy::verify_chain as verify_recovered_chain;
#[path = "workflow_task_set_publish_recover.rs"]
mod recover;
#[path = "workflow_task_set_publish_scope.rs"]
mod scope;

#[cfg(test)]
pub(crate) use journal::CRASH_ENV;
use journal::{
    Journal, JournalEntry, JournalPaths, JournalState, PublishLock, crash_point, digest_of,
    recover_journal, remove_if_present, rename, sibling_transaction_path, sync_parents,
};
pub(crate) use journal::{create_dir_all_durably, sync_parent, write_durably};
use recover::recover_before_publish;
#[cfg(test)]
pub(crate) use recover::recovery_log_path;
pub(crate) use recover::{lock_and_recover, recover_interrupted_publish};
pub(crate) use scope::{validate_destination, validate_existing_parents};

/// An exclusive advisory lock on one task set's frozen chain, keyed by its
/// pin. Every host publisher of that chain — the whole-set acceptance freeze,
/// the skeleton freeze and the per-check repair — holds it while it writes, so
/// two of them never interleave. The OS releases it when the process ends, so
/// a crash never leaves a stale lock behind, and dropping it unlocks the file
/// at once, even while a forked child still shares it (Issue 330). Acquiring
/// it first settles any publish of the set a crash interrupted.
pub(crate) struct ChainLock {
    file: std::fs::File,
}

impl Drop for ChainLock {
    fn drop(&mut self) {
        crate::command::workflow_executor_lease::release_lock(&self.file);
    }
}

impl ChainLock {
    /// Take the lock now or fail: an operator command reports the holder.
    pub(crate) fn acquire(pin_path: &Path, tasks_root: &Path) -> Result<Self> {
        let (file, path) = Self::open(pin_path)?;
        file.try_lock().map_err(|error| {
            anyhow!(
                "another freeze or repair of this task set holds {} ({error}); wait for it to finish",
                path.display()
            )
        })?;
        let lock = Self { file };
        recover_interrupted_publish(pin_path, tasks_root)?;
        Ok(lock)
    }

    /// Wait for the lock: a run's own publication outlasts a per-check repair
    /// that holds it across model calls instead of failing its stage. The OS
    /// releases a dead holder's lock, so the wait ends when the holder does;
    /// it is logged when it starts and every minute it continues.
    pub(crate) fn acquire_waiting(pin_path: &Path, tasks_root: &Path) -> Result<Self> {
        let (file, path) = Self::open(pin_path)?;
        let started = std::time::Instant::now();
        let mut reported: Option<std::time::Duration> = None;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    let waited = started.elapsed();
                    if reported.is_none_or(|last| waited - last >= CHAIN_LOCK_REPORT_EVERY) {
                        tracing::info!(
                            lock = %path.display(),
                            waited_secs = waited.as_secs(),
                            "waiting for another freeze or repair of this task set to finish"
                        );
                        reported = Some(waited);
                    }
                    std::thread::sleep(CHAIN_LOCK_POLL);
                }
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(error)
                        .with_context(|| format!("locking chain lock {}", path.display()));
                }
            }
        }
        let lock = Self { file };
        recover_interrupted_publish(pin_path, tasks_root)?;
        Ok(lock)
    }

    fn open(pin_path: &Path) -> Result<(std::fs::File, PathBuf)> {
        let path = JournalPaths::for_pin(pin_path).chain_lock;
        if let Some(parent) = path.parent() {
            create_dir_all_durably(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening chain lock {}", path.display()))?;
        Ok((file, path))
    }
}

const CHAIN_LOCK_POLL: std::time::Duration = std::time::Duration::from_millis(100);
const CHAIN_LOCK_REPORT_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

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

#[path = "workflow_task_set_publish_acceptance.rs"]
mod acceptance;
#[cfg(test)]
pub(super) use acceptance::publish_acceptance_files;
pub(super) use acceptance::publish_acceptance_files_with_recovery;

/// Publish `files` as one crash-atomic transaction journaled beside the task
/// set's acceptance pin at `pin_path`.
pub(crate) fn publish_files_atomically(
    pin_path: &Path,
    tasks_root: &Path,
    files: &[(PathBuf, Vec<u8>)],
    remedy: &str,
) -> Result<()> {
    let transaction = begin_publish(pin_path, tasks_root, files, remedy, &[])?;
    for warning in transaction.commit()? {
        tracing::warn!("{warning}");
        eprintln!("warning: {warning}");
    }
    Ok(())
}

/// A published but not yet committed freeze: every target already holds its
/// new bytes, a hard-link backup of each prior version is kept, and the
/// journal says `applying`, so a crash now restores the prior set. The caller
/// commits (the journal's atomic move to `committed`, then cleanup) or rolls
/// back. Dropped without either — a panic — the journal stays `applying` and
/// the next recovery of the set rolls it back.
#[must_use = "a publish transaction must be committed or rolled back"]
pub(crate) struct PublishTransaction {
    _lock: PublishLock,
    paths: JournalPaths,
    journal: Journal,
}

impl PublishTransaction {
    /// Pass the commit point, then durably clean up. A journal-store error
    /// leaves its on-disk decision authoritative: it may have been renamed
    /// before the directory flush failed, so never start an opposing rollback.
    pub(crate) fn commit(self) -> Result<Vec<String>> {
        let mut journal = self.journal.clone();
        journal.state = JournalState::Committed;
        journal.store(&self.paths).with_context(|| format!(
            "recording commit failed; decision in {} is kept for automatic recovery on the next acquisition",
            self.paths.journal.display()
        ))?;
        crash_point("committed");
        let backups = journal
            .entries
            .iter()
            .filter_map(|entry| {
                let backup = entry.backup.clone()?;
                Some((entry.target.clone(), backup))
            })
            .collect::<Vec<_>>();
        let mut warnings = cleanup_committed_backups(&backups, |path| {
            remove_if_present(path).map_err(std::io::Error::other)
        });
        if warnings.is_empty()
            && let Err(error) = sync_parents(backups.iter().map(|(_, backup)| backup.as_path()))
        {
            warnings.push(format!(
                "committed backup cleanup could not be flushed ({error:#}); journal kept for recovery"
            ));
        }
        if warnings.is_empty() {
            crash_point("commit-before-journal-removal");
            if let Err(error) = Journal::remove(&self.paths) {
                warnings.push(format!(
                    "freeze is committed, but its journal could not be removed ({error:#}); the next publish, launch or resume of this task set removes it"
                ));
            }
        } else {
            warnings.push(format!(
                "the committed journal {} is kept so the next publish, launch or resume of this task set finishes the cleanup",
                self.paths.journal.display()
            ));
        }
        crash_point("cleaned");
        Ok(warnings)
    }

    /// Restore every prior version (removing targets that did not exist).
    /// A target another writer replaced after this publish is theirs now and
    /// is left as it is: restoring over it would be the very lost update the
    /// chain lock exists to prevent.
    pub(crate) fn roll_back(self) -> Result<()> {
        let mut foreign = Vec::new();
        for entry in &self.journal.entries {
            match digest_of(&entry.target) {
                Ok(current) if current.as_deref() == Some(entry.written.as_str()) => {}
                Ok(_) => foreign.push(format!(
                    "{}: changed after this publish, left as it is",
                    entry.target.display()
                )),
                Err(error) => foreign.push(format!("{error:#}")),
            }
        }
        recover_journal(&self.journal, &self.paths).map_err(|error| {
            anyhow!(
                "freeze rollback failed ({error:#}); the publish journal {} is kept and the next publish, launch or resume of this task set finishes the rollback",
                self.paths.journal.display()
            )
        })?;
        if foreign.is_empty() {
            Ok(())
        } else {
            Err(anyhow!("freeze rollback skipped {}", foreign.join("; ")))
        }
    }
}

/// Roll an `applying` transaction back in-process after `error`.
fn undo(journal: &Journal, paths: &JournalPaths, error: anyhow::Error) -> anyhow::Error {
    let mut applying = journal.clone();
    applying.state = JournalState::Applying;
    match recover_journal(&applying, paths) {
        Ok(_) => error.context("the prior chain was restored"),
        Err(rollback) => error.context(format!(
            "rollback failed ({rollback:#}); the publish journal {} is kept and the next publish, launch or resume of this task set finishes the rollback",
            paths.journal.display()
        )),
    }
}

/// Abandon a transaction that replaced nothing: remove the transaction files
/// it created, then its journal. A file that cannot be removed keeps the
/// journal, so recovery retries.
fn abandon(paths: &JournalPaths, created: &[PathBuf], error: anyhow::Error) -> anyhow::Error {
    let failures = created
        .iter()
        .filter_map(|path| remove_if_present(path).err())
        .map(|failure| format!("{failure:#}"))
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        return error.context(format!(
            "cleanup failed ({}); the publish journal {} is kept for the next recovery",
            failures.join("; "),
            paths.journal.display()
        ));
    }
    if let Err(cleanup) = sync_parents(created.iter().map(PathBuf::as_path)) {
        return error.context(format!("abandoned transaction cleanup could not be flushed ({cleanup:#}); journal kept for recovery"));
    }
    match Journal::remove(paths) {
        Ok(()) => error,
        Err(cleanup) => error.context(format!("{cleanup:#}")),
    }
}

pub(crate) fn begin_publish(
    pin_path: &Path,
    tasks_root: &Path,
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
            create_dir_all_durably(parent)?;
        }
    }
    let paths = JournalPaths::for_pin(pin_path);
    let lock = PublishLock::acquire(&paths)?;
    let targets = files
        .iter()
        .map(|(target, _)| target.clone())
        .collect::<Vec<_>>();
    recover_before_publish(&paths, pin_path, tasks_root)?;
    let transaction = uuid::Uuid::new_v4().simple().to_string();
    let mut journal = Journal::new(
        transaction.clone(),
        remedy,
        files
            .iter()
            .map(|(target, bytes)| JournalEntry {
                target: target.clone(),
                staged: sibling_transaction_path(target, &transaction, "new"),
                backup: Some(sibling_transaction_path(target, &transaction, "old")),
                written: content_digest(bytes),
            })
            .collect(),
    );
    journal.validate(&paths, &recover::publication_scopes(pin_path, tasks_root)?)?;
    journal.store(&paths)?;
    crash_point("prepared");
    let mut created = Vec::new();
    let staged = (|| -> Result<()> {
        for (entry, (_, bytes)) in journal.entries.iter().zip(files) {
            write_durably(&entry.staged, bytes).map_err(|error| {
                // A partly written file is still ours to remove.
                created.push(entry.staged.clone());
                anyhow!(error).context(format!("writing {}", entry.staged.display()))
            })?;
            created.push(entry.staged.clone());
        }
        sync_parents(targets.iter().map(PathBuf::as_path))
    })();
    if let Err(error) = staged {
        return Err(abandon(&paths, &created, error));
    }
    crash_point("staged");
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
        let error = anyhow!(
            "the frozen chain changed since it was verified ({}); nothing was written — re-run against the current chain",
            changed.join(", ")
        );
        return Err(abandon(&paths, &created, error));
    }
    let backed_up = (|| -> Result<()> {
        for entry in &mut journal.entries {
            let Some(backup) = entry.backup.clone() else {
                continue;
            };
            if !entry.target.exists() {
                entry.backup = None;
                continue;
            }
            std::fs::hard_link(&entry.target, &backup).with_context(|| {
                format!("backing up {} before publishing", entry.target.display())
            })?;
            created.push(backup);
        }
        sync_parents(targets.iter().map(PathBuf::as_path))?;
        journal.state = JournalState::Applying;
        journal.store(&paths)
    })();
    if let Err(error) = backed_up {
        return Err(abandon(&paths, &created, error));
    }
    crash_point("applying");
    let applied = (|| -> Result<()> {
        for (index, entry) in journal.entries.iter().enumerate() {
            rename(&entry.staged, &entry.target)
                .with_context(|| format!("publishing freeze to {}", entry.target.display()))?;
            crash_point(&format!("renamed-{index}"));
        }
        sync_parents(targets.iter().map(PathBuf::as_path))
    })();
    if let Err(error) = applied {
        return Err(undo(&journal, &paths, error));
    }
    Ok(PublishTransaction {
        _lock: lock,
        paths,
        journal,
    })
}

pub(super) fn cleanup_committed_backups<F>(
    backups: &[(PathBuf, PathBuf)],
    mut remove: F,
) -> Vec<String>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let mut warnings = Vec::new();
    for (index, (_, backup)) in backups.iter().enumerate() {
        if let Err(error) = remove(backup) {
            warnings.push(format!(
                "freeze is already committed, but transaction backup {} could not be removed: {error}; verify the live freeze, then remove the stale backup manually",
                backup.display()
            ));
        }
        crash_point(&format!("commit-backup-cleaned-{index}"));
    }
    warnings
}

#[cfg(test)]
#[path = "workflow_task_set_publish_crash_tests.rs"]
mod crash_tests;

#[cfg(test)]
#[path = "workflow_task_set_publish_shared_lock_tests.rs"]
mod shared_lock_tests;
#[cfg(test)]
pub(crate) use shared_lock_tests::stick_next_commit;

#[cfg(test)]
pub(crate) fn reader_test_step(step: &str) {
    crash_point(step);
}
