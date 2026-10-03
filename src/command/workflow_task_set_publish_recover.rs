//! Automatic recovery of a task-set publish a crash interrupted (Issue 271).
//!
//! Run under the set's publish lock by every publisher before it writes, by
//! the chain lock's acquisition, and at run launch and resume, so nothing ever
//! trusts a set a killed publish left half replaced. Each recovery is logged
//! (tracing) and appended to a durable recovery log beside the pin.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::journal::{
    Journal, JournalPaths, PublishLock, is_transaction_id, recover_journal, remove_if_present,
    rename, sync_parents,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryOutcome {
    /// The interrupted publish never reached its commit point: the complete
    /// old set is live again.
    RolledBack,
    /// The publish had passed its commit point: the complete new set is live.
    RolledForward,
}

impl RecoveryOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::RolledBack => "rolled back",
            Self::RolledForward => "rolled forward",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecoveryEvent {
    pub(crate) transaction: String,
    /// `journal` for this binary's journaled publish; `legacy` for the
    /// transaction files of a binary that published without a journal.
    pub(crate) source: &'static str,
    pub(crate) outcome: RecoveryOutcome,
    /// The targets restored, removed or completed.
    pub(crate) files: Vec<PathBuf>,
}

#[derive(Debug, Default)]
pub(crate) struct RecoveryReport {
    pub(crate) events: Vec<RecoveryEvent>,
}

#[cfg(test)]
pub(crate) fn recovery_log_path(pin_path: &Path) -> PathBuf {
    JournalPaths::for_pin(pin_path).log
}

/// Settle any publish of the task set at `tasks_root` (pinned at `pin_path`)
/// that a crash interrupted. Waits for a live publisher to finish first.
pub(crate) fn recover_interrupted_publish(
    pin_path: &Path,
    tasks_root: &Path,
) -> Result<RecoveryReport> {
    let paths = JournalPaths::for_pin(pin_path);
    let _lock = PublishLock::acquire(&paths)?;
    recover_locked(&paths, &legacy_scopes(pin_path, tasks_root))
}

/// The places a journal-less publish of this set left transaction files: the
/// task directory (only this set's files live there), and the pin and its
/// check-sources sidecar, which share their directories with other sets and
/// so are matched by this set's file name only.
fn legacy_scopes(pin_path: &Path, tasks_root: &Path) -> Vec<(PathBuf, Option<String>)> {
    let mut scopes = vec![(tasks_root.to_path_buf(), None)];
    if let (Some(dir), Some(name)) = (pin_path.parent(), pin_path.file_name()) {
        let name = name.to_string_lossy().into_owned();
        scopes.push((dir.to_path_buf(), Some(name.clone())));
        scopes.push((dir.join("check-sources"), Some(name)));
    }
    scopes
}

/// Recovery for a publisher about to write `targets`, already holding the
/// publish lock: the journal, then legacy files beside exactly those targets.
pub(super) fn recover_before_publish(
    paths: &JournalPaths,
    targets: &[PathBuf],
) -> Result<RecoveryReport> {
    let scopes = targets
        .iter()
        .filter_map(|target| {
            let dir = target.parent()?.to_path_buf();
            let name = target.file_name()?.to_string_lossy().into_owned();
            Some((dir, Some(name)))
        })
        .collect::<Vec<_>>();
    recover_locked(paths, &scopes)
}

fn recover_locked(
    paths: &JournalPaths,
    scopes: &[(PathBuf, Option<String>)],
) -> Result<RecoveryReport> {
    let mut report = RecoveryReport::default();
    if let Some(journal) = Journal::load(paths)? {
        let recovered = recover_journal(&journal, paths).with_context(|| {
            format!(
                "recovering the interrupted publish recorded in {}",
                paths.journal.display()
            )
        })?;
        report.events.push(RecoveryEvent {
            transaction: journal.transaction,
            source: "journal",
            outcome: if recovered.rolled_forward {
                RecoveryOutcome::RolledForward
            } else {
                RecoveryOutcome::RolledBack
            },
            files: recovered.files,
        });
    }
    // A crash while a journal state was being written leaves only its temp;
    // the journal it was replacing (if any) was authoritative and is settled.
    remove_if_present(&paths.journal_temp())?;
    report.events.extend(recover_legacy(scopes)?);
    for event in &report.events {
        record(paths, event);
    }
    Ok(report)
}

/// One journal-less transaction's files, by transaction id.
#[derive(Default)]
struct LegacyTransaction {
    staged: Vec<(PathBuf, PathBuf)>,
    backups: Vec<PathBuf>,
}

/// A binary without the journal staged and fsynced every file of a publish
/// before its first rename, and backed each prior version up immediately
/// before replacing it. So a transaction with any backup had finished
/// staging: its staged files are whole and it is rolled forward. One with no
/// backup had replaced nothing that existed: its staged files are discarded.
fn recover_legacy(scopes: &[(PathBuf, Option<String>)]) -> Result<Vec<RecoveryEvent>> {
    let mut transactions = BTreeMap::<String, LegacyTransaction>::new();
    for (dir, only) in scopes {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("listing {}", dir.display()));
            }
        };
        for entry in entries {
            let entry = entry.with_context(|| format!("listing {}", dir.display()))?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let Some((target, transaction, role)) = parse_transaction_file(&file_name) else {
                continue;
            };
            if only.as_deref().is_some_and(|only| only != target) {
                continue;
            }
            let found = transactions.entry(transaction.to_string()).or_default();
            match role {
                "new" => found.staged.push((entry.path(), dir.join(target))),
                _ => found.backups.push(entry.path()),
            }
        }
    }
    let mut events = Vec::new();
    for (transaction, found) in transactions {
        let forward = !found.backups.is_empty();
        let mut files = Vec::new();
        for (staged, target) in &found.staged {
            if forward {
                rename(staged, target)?;
                files.push(target.clone());
            } else {
                remove_if_present(staged)?;
                files.push(staged.clone());
            }
        }
        for backup in &found.backups {
            remove_if_present(backup)?;
        }
        let touched = found.staged.iter().map(|(staged, _)| staged.as_path());
        sync_parents(touched.chain(found.backups.iter().map(PathBuf::as_path)))?;
        events.push(RecoveryEvent {
            transaction,
            source: "legacy",
            outcome: if forward {
                RecoveryOutcome::RolledForward
            } else {
                RecoveryOutcome::RolledBack
            },
            files,
        });
    }
    Ok(events)
}

/// `.<target>.<32-hex transaction>.<new|old>` → (target, transaction, role).
fn parse_transaction_file(file_name: &str) -> Option<(&str, &str, &str)> {
    let rest = file_name.strip_prefix('.')?;
    let (rest, role) = rest.rsplit_once('.')?;
    let (target, transaction) = rest.rsplit_once('.')?;
    (matches!(role, "new" | "old") && is_transaction_id(transaction) && !target.is_empty())
        .then_some((target, transaction, role))
}

/// Log the recovery and append it to the durable recovery log. The set is
/// already settled here, so a log that cannot be appended to is reported as
/// an error in the trace rather than failing the caller.
fn record(paths: &JournalPaths, event: &RecoveryEvent) {
    let files = event
        .files
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    tracing::warn!(
        transaction = %event.transaction,
        source = event.source,
        outcome = event.outcome.as_str(),
        files = ?files,
        "recovered an interrupted task-set publish"
    );
    let line = serde_json::json!({
        "event": "task_set_publish_recovered",
        "at": chrono::Utc::now().to_rfc3339(),
        "transaction": event.transaction,
        "source": event.source,
        "outcome": event.outcome.as_str(),
        "files": files,
    });
    let appended = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
        .and_then(|mut log| writeln!(log, "{line}").and_then(|()| log.sync_all()));
    if let Err(error) = appended {
        tracing::error!(
            log = %paths.log.display(),
            %error,
            "could not append the task-set publish recovery to its log"
        );
    }
}
