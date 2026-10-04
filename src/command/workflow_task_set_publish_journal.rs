//! The durable intent journal behind every task-set publish (Issue 271).
//!
//! A publish replaces several files in more than one directory, so no single
//! rename can swap them all. Instead one small journal file, kept beside the
//! task set's acceptance pin, records the whole transaction, and every change
//! of its state is one atomic `rename` of a fully written, fsynced temp over
//! it, followed by an fsync of its directory:
//!
//! - `prepared`: the staged and backup paths are named; no target is touched.
//! - `applying`: every staged file is whole and durable and every prior
//!   version has a hard-link backup; targets are being replaced.
//! - `committed`: the commit point. The new set is final; only cleanup is left.
//!
//! Recovery, under the same publish lock every publisher holds, rolls a
//! `prepared` or `applying` journal back to the complete old set and a
//! `committed` journal forward to the complete new set, then removes it.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::content_digest;
use serde::{Deserialize, Serialize};

const JOURNAL_SCHEMA_VERSION: u32 = 1;

/// Test-only fault injection: the named step aborts the process on the spot.
#[cfg(test)]
pub(crate) const CRASH_ENV: &str = "ARCHON_TEST_PUBLISH_CRASH_AT";

/// A crash point in the publish protocol. Compiled to nothing outside tests;
/// in tests the process kills itself — SIGKILL on Unix, abort elsewhere: no
/// destructors, no cleanup — when `CRASH_ENV` names the step.
pub(super) fn crash_point(step: &str) {
    #[cfg(test)]
    if std::env::var(CRASH_ENV).ok().as_deref() == Some(step) {
        #[cfg(unix)]
        // SAFETY: signalling our own pid has no memory-safety preconditions.
        unsafe {
            libc::kill(libc::getpid(), libc::SIGKILL);
        }
        std::process::abort();
    }
    #[cfg(test)]
    test_hooks::step(step);
    let _ = step;
}

/// Where one task set's publish lock, journal and recovery log live: beside
/// its acceptance pin, so every publisher and reader of the set derives them.
#[derive(Debug, Clone)]
pub(crate) struct JournalPaths {
    pub(super) lock: PathBuf,
    pub(super) journal: PathBuf,
    pub(super) log: PathBuf,
}

impl JournalPaths {
    pub(crate) fn for_pin(pin_path: &Path) -> Self {
        Self {
            lock: pin_path.with_extension("publish.lock"),
            journal: pin_path.with_extension("publish-journal"),
            log: pin_path.with_extension("publish-recovery.log"),
        }
    }

    pub(super) fn journal_temp(&self) -> PathBuf {
        let mut name = self.journal.as_os_str().to_owned();
        name.push(".tmp");
        PathBuf::from(name)
    }
}

/// The exclusive publish lock of one task set. Held for the whole life of a
/// publish transaction and by every recovery, so a recovery never undoes a
/// live publisher's work. The OS releases it when its holder dies.
pub(crate) struct PublishLock {
    _file: std::fs::File,
}

impl PublishLock {
    /// Blocks until the lock is free: a holder keeps it only while it writes
    /// and verifies one transaction.
    pub(super) fn acquire(paths: &JournalPaths) -> Result<Self> {
        if let Some(parent) = paths.lock.parent() {
            create_dir_all_durably(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&paths.lock)
            .with_context(|| format!("opening publish lock {}", paths.lock.display()))?;
        file.lock()
            .with_context(|| format!("locking publish lock {}", paths.lock.display()))?;
        Ok(Self { _file: file })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum JournalState {
    Prepared,
    Applying,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct JournalEntry {
    pub(super) target: PathBuf,
    pub(super) staged: PathBuf,
    /// The hard-link backup of the prior version: always named while
    /// `prepared`; from `applying` on, present only when a prior existed.
    pub(super) backup: Option<PathBuf>,
    /// Digest of the bytes this transaction writes to `target`.
    pub(super) written: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Journal {
    pub(super) schema_version: u32,
    pub(super) transaction: String,
    pub(super) state: JournalState,
    pub(super) remedy: String,
    pub(super) entries: Vec<JournalEntry>,
}

impl Journal {
    pub(super) fn new(transaction: String, remedy: &str, entries: Vec<JournalEntry>) -> Self {
        Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            transaction,
            state: JournalState::Prepared,
            remedy: remedy.to_string(),
            entries,
        }
    }

    /// Durably replace the journal on disk with this state: one atomic rename.
    pub(super) fn store(&self, paths: &JournalPaths) -> Result<()> {
        let temp = paths.journal_temp();
        write_durably(&temp, &serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("writing publish journal {}", temp.display()))?;
        std::fs::rename(&temp, &paths.journal)
            .with_context(|| format!("recording publish journal {}", paths.journal.display()))?;
        crash_point(&format!("journal-replaced-{:?}", self.state));
        #[cfg(test)]
        if std::env::var("ARCHON_TEST_COMMIT_FLUSH_ERROR").is_ok()
            && self.state == JournalState::Committed
        {
            return Err(anyhow!("injected commit directory flush failure"));
        }
        sync_parent(&paths.journal)?;
        crash_point(&format!("journal-flushed-{:?}", self.state));
        Ok(())
    }

    /// The journal left on disk, if any, validated before any path in it is
    /// trusted: it is data read back from disk.
    pub(super) fn load(
        paths: &JournalPaths,
        scopes: &super::scope::Scopes,
    ) -> Result<Option<Self>> {
        let bytes = match std::fs::read(&paths.journal) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("reading publish journal {}", paths.journal.display())
                });
            }
        };
        let journal: Self = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "publish journal {} is not readable; inspect it and the transaction files it names, then remove it",
                paths.journal.display()
            )
        })?;
        journal.validate(paths, scopes)?;
        Ok(Some(journal))
    }

    pub(super) fn validate(
        &self,
        paths: &JournalPaths,
        scopes: &super::scope::Scopes,
    ) -> Result<()> {
        if self.schema_version != JOURNAL_SCHEMA_VERSION || !is_transaction_id(&self.transaction) {
            return Err(anyhow!(
                "publish journal {} has schema {} / transaction {:?}, which this binary does not write; inspect it, then remove it",
                paths.journal.display(),
                self.schema_version,
                self.transaction
            ));
        }
        let mut targets = std::collections::BTreeSet::new();
        for entry in &self.entries {
            let expected = [
                Some((&entry.staged, "new")),
                entry.backup.as_ref().map(|backup| (backup, "old")),
            ];
            for (path, role) in expected.into_iter().flatten() {
                if *path != sibling_transaction_path(&entry.target, &self.transaction, role) {
                    return Err(anyhow!(
                        "publish journal {} names {}, which is not a transaction file of {}; nothing was changed — inspect the journal, then remove it",
                        paths.journal.display(),
                        path.display(),
                        entry.target.display()
                    ));
                }
            }
            let target = super::scope::validate_destination(&entry.target, scopes)?;
            let reserved = [
                paths.lock.clone(),
                paths.lock.with_extension("chain.lock"),
                paths.journal.clone(),
                paths.log.clone(),
                paths.journal_temp(),
            ]
            .iter()
            .any(|path| {
                path.parent()
                    .and_then(|parent| parent.canonicalize().ok())
                    .as_deref()
                    == target.parent()
                    && path.file_name() == target.file_name()
            });
            if reserved || !targets.insert(target) {
                return Err(anyhow!(
                    "publish journal has duplicate or reserved target {}",
                    entry.target.display()
                ));
            }
            super::scope::validate_destination(&entry.staged, &transaction_scopes(&entry.target))?;
            if let Some(backup) = &entry.backup {
                super::scope::validate_destination(backup, &transaction_scopes(&entry.target))?;
            }
        }
        Ok(())
    }

    pub(super) fn remove(paths: &JournalPaths) -> Result<()> {
        match std::fs::remove_file(&paths.journal) {
            Ok(()) => sync_parent(&paths.journal),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error)
                .with_context(|| format!("removing publish journal {}", paths.journal.display())),
        }
    }
}

fn transaction_scopes(target: &Path) -> Vec<(PathBuf, Option<String>)> {
    target
        .parent()
        .map(|parent| vec![(parent.to_path_buf(), None)])
        .unwrap_or_default()
}

/// What recovering one journal did, for the recovery log.
pub(super) struct Recovered {
    pub(super) rolled_forward: bool,
    pub(super) files: Vec<PathBuf>,
}

/// Bring the set to the state the journal's commit point decides: back to the
/// complete old set before it, forward to the complete new set after it.
/// Idempotent, and the journal is removed only once every file is settled, so
/// a crash during recovery is recovered again from the same journal.
pub(super) fn recover_journal(journal: &Journal, paths: &JournalPaths) -> Result<Recovered> {
    let files = match journal.state {
        JournalState::Prepared => Vec::new(),
        JournalState::Applying => roll_back_entries(&journal.entries)?,
        JournalState::Committed => roll_forward_entries(&journal.entries)?,
    };
    // Settle target renames durably before consuming any remaining backups.
    sync_parents(journal.entries.iter().map(|entry| entry.target.as_path()))?;
    for (index, entry) in journal.entries.iter().enumerate() {
        remove_if_present(&entry.staged)?;
        if let Some(backup) = &entry.backup {
            remove_if_present(backup)?;
        }
        crash_point(&format!("backup-cleaned-{index}"));
    }
    sync_parents(journal.entries.iter().map(|entry| entry.target.as_path()))?;
    let recovered = Recovered {
        rolled_forward: journal.state == JournalState::Committed,
        files,
    };
    super::recover::record_journal(paths, journal, &recovered)?;
    crash_point("before-journal-removal");
    Journal::remove(paths)?;
    Ok(recovered)
}

/// Undo this transaction's replacements, newest first. A target that does
/// not hold the bytes this transaction wrote was never replaced (the crash
/// came first) or was replaced by someone else since; either way it is left.
/// Returns the targets restored or removed.
pub(super) fn roll_back_entries(entries: &[JournalEntry]) -> Result<Vec<PathBuf>> {
    let mut undone = Vec::new();
    for (index, entry) in entries.iter().rev().enumerate() {
        let current = digest_of(&entry.target)?;
        let ours = current.as_deref() == Some(entry.written.as_str());
        let backup = entry.backup.as_ref().filter(|backup| backup.exists());
        match (ours, backup) {
            (true, Some(backup)) => rename(backup, &entry.target)?,
            (true, None) if entry.backup.is_some() => continue,
            (true, None) => std::fs::remove_file(&entry.target)
                .with_context(|| format!("removing {}", entry.target.display()))?,
            (false, Some(backup)) if current.is_none() => rename(backup, &entry.target)?,
            (false, None) if entry.backup.is_some() && current.is_none() => {
                return Err(anyhow!(
                    "missing target and prior backup for {}",
                    entry.target.display()
                ));
            }
            (false, _) => continue,
        }
        crash_point(&format!("restored-{index}"));
        undone.push(entry.target.clone());
    }
    Ok(undone)
}

/// Finish replacing every target the transaction had not reached yet.
fn roll_forward_entries(entries: &[JournalEntry]) -> Result<Vec<PathBuf>> {
    // Prove the entire new set is available before changing any target.
    for entry in entries {
        if digest_of(&entry.target)?.as_deref() != Some(entry.written.as_str())
            && digest_of(&entry.staged)?.as_deref() != Some(entry.written.as_str())
        {
            return Err(anyhow!(
                "committed publication is incomplete: {} and {} do not hold the recorded bytes; journal and backups kept",
                entry.target.display(),
                entry.staged.display()
            ));
        }
    }
    let mut finished = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if digest_of(&entry.target)?.as_deref() != Some(entry.written.as_str()) {
            rename(&entry.staged, &entry.target)?;
            crash_point(&format!("forwarded-{index}"));
        }
        finished.push(entry.target.clone());
    }
    Ok(finished)
}

pub(super) fn digest_of(path: &Path) -> Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(content_digest(&bytes))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

pub(super) fn rename(from: &Path, to: &Path) -> Result<()> {
    std::fs::rename(from, to)
        .with_context(|| format!("renaming {} to {}", from.display(), to.display()))
}

pub(super) fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

/// Write `bytes` to a new file and flush them to stable storage through the
/// same write handle (Windows' FlushFileBuffers requires write access).
pub(crate) fn write_durably(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Connect every created directory durably to its parent. Existing ancestors
/// are flushed too: a caller may just have created them without flushing them.
pub(crate) fn create_dir_all_durably(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for ancestor in dir.ancestors().filter(|path| !path.as_os_str().is_empty()) {
        sync_dir(ancestor)?;
    }
    Ok(())
}

/// Flush a directory's entries, so a rename or unlink in it is durable.
pub(super) fn sync_dir(dir: &Path) -> Result<()> {
    #[cfg(test)]
    test_hooks::synced(dir);
    open_dir_for_sync(dir)
        .and_then(|handle| handle.sync_all())
        .with_context(|| format!("flushing directory {}", dir.display()))
}

#[cfg(not(windows))]
fn open_dir_for_sync(dir: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(dir)
}

#[cfg(windows)]
fn open_dir_for_sync(dir: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    // A directory handle needs FILE_FLAG_BACKUP_SEMANTICS, and
    // FlushFileBuffers needs it opened with write access.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)
}

pub(crate) fn sync_parent(path: &Path) -> Result<()> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => sync_dir(parent),
        _ => sync_dir(Path::new(".")),
    }
}

/// Flush each distinct parent directory of `paths` once.
pub(crate) fn sync_parents<'a>(paths: impl Iterator<Item = &'a Path>) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for path in paths {
        if seen.insert(path.parent().map(Path::to_path_buf)) {
            sync_parent(path)?;
        }
    }
    Ok(())
}

pub(super) fn sibling_transaction_path(target: &Path, transaction: &str, role: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    target.with_file_name(format!(".{name}.{transaction}.{role}"))
}

pub(super) fn is_transaction_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
#[path = "workflow_task_set_publish_test_hooks.rs"]
pub(super) mod test_hooks;
