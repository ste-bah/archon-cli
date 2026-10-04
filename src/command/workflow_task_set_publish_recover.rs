//! Automatic recovery of a task-set publish a crash interrupted (Issue 271).
//!
//! Run under the set's publish lock by every publisher before it writes, by
//! the chain lock's acquisition, and at run launch and resume, so nothing ever
//! trusts a set a killed publish left half replaced. Each recovery is logged
//! (tracing) and appended to a durable recovery log beside the pin.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use super::journal::{
    Journal, JournalPaths, PublishLock, Recovered, crash_point, is_transaction_id, recover_journal,
    remove_if_present, sync_parent,
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
    let (_lock, report) = lock_and_recover(pin_path, tasks_root)?;
    Ok(report)
}

/// Keep the same publish lock through the caller's complete read.
pub(crate) fn lock_and_recover(
    pin_path: &Path,
    tasks_root: &Path,
) -> Result<(PublishLock, RecoveryReport)> {
    let paths = JournalPaths::for_pin(pin_path);
    let lock = PublishLock::acquire(&paths)?;
    let report = recover_locked(&paths, &publication_scopes(pin_path, tasks_root)?)?;
    Ok((lock, report))
}

pub(super) fn publication_scopes(
    pin_path: &Path,
    tasks_root: &Path,
) -> Result<Vec<(PathBuf, Option<String>)>> {
    let mut scopes = legacy_scopes(pin_path, tasks_root);
    scopes.extend(
        super::super::super::workflow_host_command_publish::receipt_scopes(pin_path, tasks_root)?,
    );
    Ok(scopes)
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

/// Recovery under the publish lock uses the complete trusted task-set scope,
/// including files an earlier publisher wrote outside this caller's subset.
pub(super) fn recover_before_publish(
    paths: &JournalPaths,
    pin_path: &Path,
    tasks_root: &Path,
) -> Result<RecoveryReport> {
    recover_locked(paths, &publication_scopes(pin_path, tasks_root)?)
}

fn recover_locked(
    paths: &JournalPaths,
    scopes: &[(PathBuf, Option<String>)],
) -> Result<RecoveryReport> {
    let mut report = RecoveryReport::default();
    if let Some(journal) = Journal::load(paths, scopes)? {
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
    let temp = paths.journal_temp();
    match std::fs::read(&temp) {
        Ok(bytes) => {
            let event = RecoveryEvent {
                transaction: archon_workflow::task_set_contract::content_digest(&bytes),
                source: "journal_temp",
                outcome: RecoveryOutcome::RolledBack,
                files: vec![temp.clone()],
            };
            record(paths, &event)?;
            remove_if_present(&temp)?;
            sync_parent(&temp)?;
            report.events.push(event);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("reading {}", temp.display())),
    }
    report.events.extend(recover_legacy(scopes)?);
    Ok(report)
}

/// One journal-less transaction's files, by transaction id.
#[derive(Default)]
struct LegacyTransaction {
    staged: Vec<(PathBuf, PathBuf)>,
    backups: Vec<PathBuf>,
}

/// Without a durable manifest neither a backup nor surviving staging proves
/// the transaction's full membership, original absences, or new contents.
/// Preserve every ambiguous legacy transaction as corruption, naming its files.
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
    if transactions.is_empty() {
        return Ok(Vec::new());
    }
    let evidence = transactions
        .into_iter()
        .map(|(transaction, found)| {
            let files = found
                .staged
                .iter()
                .map(|(path, _)| path)
                .chain(found.backups.iter())
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>();
            format!("{transaction}: {}", files.join(", "))
        })
        .collect::<Vec<_>>();
    Err(anyhow!(
        "ambiguous legacy publication debris: {}; no durable manifest proves a complete old or new set; files kept for inspection",
        evidence.join("; ")
    ))
}

/// `.<target>.<32-hex transaction>.<new|old>` → (target, transaction, role).
fn parse_transaction_file(file_name: &str) -> Option<(&str, &str, &str)> {
    let rest = file_name.strip_prefix('.')?;
    let (rest, role) = rest.rsplit_once('.')?;
    let (target, transaction) = rest.rsplit_once('.')?;
    (matches!(role, "new" | "old") && is_transaction_id(transaction) && !target.is_empty())
        .then_some((target, transaction, role))
}

pub(super) fn record_journal(
    paths: &JournalPaths,
    journal: &Journal,
    recovered: &Recovered,
) -> Result<()> {
    record(
        paths,
        &RecoveryEvent {
            transaction: journal.transaction.clone(),
            source: "journal",
            outcome: if recovered.rolled_forward {
                RecoveryOutcome::RolledForward
            } else {
                RecoveryOutcome::RolledBack
            },
            files: recovered.files.clone(),
        },
    )
}

/// A durable record precedes journal removal. Failure retains the decision so
/// the next acquisition retries; a crash may produce duplicate events.
fn record(paths: &JournalPaths, event: &RecoveryEvent) -> Result<()> {
    super::journal::crash_point("before-recovery-log");
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
    // A delimiter also keeps a retry's event readable after a torn append.
    super::scope::validate_destination(
        &paths.log,
        &[(
            paths
                .log
                .parent()
                .ok_or_else(|| anyhow!("recovery log has no parent"))?
                .to_path_buf(),
            None,
        )],
    )?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
        .and_then(|mut log| writeln!(log, "\n{line}").and_then(|()| log.sync_all()))
        .with_context(|| {
            format!(
                "recording recovery in {}; journal kept for retry",
                paths.log.display()
            )
        })?;
    sync_parent(&paths.log)?;
    crash_point("after-recovery-log");
    Ok(())
}
