//! Self-healing recovery of the transaction files a publisher without the
//! journal left behind (Issue 271).
//!
//! That publisher (main before this journal) staged and fsynced every
//! `.<name>.<txn>.new` before its first rename, backed each prior version up
//! as `.<name>.<txn>.old` just before replacing it, and also left a `.old`
//! when a backup or rollback cleanup failed. Its debris is therefore always
//! rolled forward: every staged file is moved into place and every backup is
//! dropped. Because that evidence carries no manifest, the frozen chain is
//! then verified; a chain that does not verify is treated as not frozen — its
//! locks and pin are moved aside, kept for inspection — so the workflow
//! re-freezes the set. Nothing here refuses a run, and every step is logged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, AcceptancePin,
    TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
};

use super::journal::{
    JournalPaths, digest_of, is_transaction_id, remove_if_present, rename, sync_parents,
};
use super::recover::{RecoveryEvent, RecoveryOutcome, record};

/// One journal-less transaction's files, by transaction id.
#[derive(Default)]
struct LegacyTransaction {
    staged: Vec<(PathBuf, PathBuf)>,
    backups: Vec<PathBuf>,
}

/// Roll every legacy transaction in `scopes` forward, then verify the chain
/// of the set pinned at `pin_path` if any of them touched it.
pub(super) fn recover_legacy(
    paths: &JournalPaths,
    scopes: &[(PathBuf, Option<String>)],
    pin_path: &Path,
    tasks_root: &Path,
) -> Result<Vec<RecoveryEvent>> {
    let chain = chain_files(pin_path, tasks_root);
    let mut events = Vec::new();
    let mut touched_chain = None;
    for (transaction, found) in collect(scopes)? {
        let mut files = Vec::new();
        for (staged, target) in &found.staged {
            rename(staged, target)?;
            files.push(target.clone());
        }
        let backups = found.backups.iter().map(PathBuf::as_path);
        let staged = found.staged.iter().map(|(_, target)| target.as_path());
        sync_parents(staged.chain(backups.clone()))?;
        let event = RecoveryEvent {
            transaction: transaction.clone(),
            source: "legacy",
            outcome: RecoveryOutcome::RolledForward,
            files,
            detail: None,
        };
        // The durable record precedes dropping the last evidence.
        record(paths, &event)?;
        for backup in &found.backups {
            remove_if_present(backup)?;
        }
        sync_parents(backups)?;
        let names_chain = |path: &Path| chain.iter().any(|file| same_file_name(file, path));
        if found.staged.iter().any(|(_, target)| names_chain(target))
            || found
                .backups
                .iter()
                .any(|backup| names_chain(&target_of(backup)))
        {
            touched_chain.get_or_insert(transaction);
        }
        events.push(event);
    }
    // An unfreeze a crash interrupted left some of the chain moved aside:
    // verify again and finish it (a set re-frozen since verifies and stays).
    let interrupted = has_unverified(&chain)?;
    if (touched_chain.is_some() || interrupted)
        && let Err(reason) = verify_chain(pin_path, tasks_root)
    {
        let transaction =
            touched_chain.unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        events.push(unfreeze(
            paths,
            pin_path,
            tasks_root,
            &transaction,
            &reason,
        )?);
    }
    Ok(events)
}

const UNVERIFIED: &str = ".unverified-";

/// Whether any chain file has a moved-aside `.<name>.unverified-<id>` sibling.
fn has_unverified(chain: &[PathBuf]) -> Result<bool> {
    for file in chain {
        let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
            continue;
        };
        let prefix = format!(".{}{UNVERIFIED}", name.to_string_lossy());
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).with_context(|| format!("listing {}", dir.display())),
        };
        for entry in entries {
            let entry = entry.with_context(|| format!("listing {}", dir.display()))?;
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn collect(scopes: &[(PathBuf, Option<String>)]) -> Result<BTreeMap<String, LegacyTransaction>> {
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
    Ok(transactions)
}

/// `.<target>.<32-hex transaction>.<new|old>` → (target, transaction, role).
pub(super) fn parse_transaction_file(file_name: &str) -> Option<(&str, &str, &str)> {
    let rest = file_name.strip_prefix('.')?;
    let (rest, role) = rest.rsplit_once('.')?;
    let (target, transaction) = rest.rsplit_once('.')?;
    (matches!(role, "new" | "old") && is_transaction_id(transaction) && !target.is_empty())
        .then_some((target, transaction, role))
}

fn target_of(transaction_file: &Path) -> PathBuf {
    let name = transaction_file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    match parse_transaction_file(&name) {
        Some((target, _, _)) => transaction_file.with_file_name(target),
        None => transaction_file.to_path_buf(),
    }
}

fn same_file_name(left: &Path, right: &Path) -> bool {
    left.file_name() == right.file_name() && left.parent() == right.parent()
}

/// The chain files of the set: the four under the task root, the pin and its
/// check-sources sidecar (named like the pin, as `PinStore::frozen` keys it).
fn chain_files(pin_path: &Path, tasks_root: &Path) -> Vec<PathBuf> {
    let mut files = [
        ACCEPTANCE_CONTRACT_FILE,
        ACCEPTANCE_LOCK_FILE,
        TASK_SKELETON_FILE,
        TASK_SKELETON_LOCK_FILE,
    ]
    .iter()
    .map(|name| tasks_root.join(name))
    .collect::<Vec<_>>();
    files.push(pin_path.to_path_buf());
    if let Some(sidecar) = sidecar_path(pin_path) {
        files.push(sidecar);
    }
    files
}

fn sidecar_path(pin_path: &Path) -> Option<PathBuf> {
    Some(
        pin_path
            .parent()?
            .join("check-sources")
            .join(pin_path.file_name()?),
    )
}

/// The digest bindings every reader checks: each lock against its file, the
/// pin against both, the task root, and the check-sources sidecar. `Err`
/// carries the first mismatch.
fn verify_chain(pin_path: &Path, tasks_root: &Path) -> std::result::Result<(), String> {
    let acceptance_lock = tasks_root.join(ACCEPTANCE_LOCK_FILE);
    let skeleton_lock = tasks_root.join(TASK_SKELETON_LOCK_FILE);
    if !acceptance_lock.exists() {
        return match skeleton_lock.exists() {
            true => Err(format!(
                "{TASK_SKELETON_LOCK_FILE} without {ACCEPTANCE_LOCK_FILE}"
            )),
            false => Ok(()),
        };
    }
    let read_json = |path: &Path| -> std::result::Result<serde_json::Value, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
    };
    let pin: AcceptancePin = serde_json::from_value(read_json(pin_path)?)
        .map_err(|error| format!("{}: {error}", pin_path.display()))?;
    let lock: AcceptanceLock = serde_json::from_value(read_json(&acceptance_lock)?)
        .map_err(|error| format!("{}: {error}", acceptance_lock.display()))?;
    let contract = file_digest(&tasks_root.join(ACCEPTANCE_CONTRACT_FILE))?;
    if contract.as_deref() != Some(lock.digest.as_str()) || pin.acceptance_digest != lock.digest {
        return Err("acceptance contract, lock and pin digests differ".into());
    }
    if skeleton_lock.exists() {
        archon_workflow::task_skeleton::validate_full_chain(tasks_root, &pin)
            .map_err(|error| error.to_string())?;
    } else if pin.skeleton_digest.is_some() {
        return Err(format!(
            "pin records a skeleton but {TASK_SKELETON_LOCK_FILE} is absent"
        ));
    }
    if let Some(expected) = &pin.check_sources_digest {
        let sidecar = sidecar_path(pin_path).ok_or("pin has no sidecar path")?;
        if file_digest(&sidecar)?.as_deref() != Some(expected.as_str()) {
            return Err(format!("{} does not match its pin", sidecar.display()));
        }
    }
    Ok(())
}

fn file_digest(path: &Path) -> std::result::Result<Option<String>, String> {
    digest_of(path).map_err(|error| format!("{error:#}"))
}

/// Treat the set as not frozen: move its locks, pin and sidecar aside under a
/// name no reader or recovery matches, so the next freeze stage re-freezes it.
fn unfreeze(
    paths: &JournalPaths,
    pin_path: &Path,
    tasks_root: &Path,
    transaction: &str,
    reason: &str,
) -> Result<RecoveryEvent> {
    let mut moved = Vec::new();
    let mut targets = vec![
        tasks_root.join(ACCEPTANCE_LOCK_FILE),
        tasks_root.join(TASK_SKELETON_LOCK_FILE),
        pin_path.to_path_buf(),
    ];
    targets.extend(sidecar_path(pin_path));
    for target in targets.iter().filter(|target| target.exists()) {
        let name = target
            .file_name()
            .ok_or_else(|| anyhow!("{} has no file name", target.display()))?
            .to_string_lossy();
        let aside = target.with_file_name(format!(".{name}{UNVERIFIED}{transaction}"));
        rename(target, &aside)?;
        moved.push(aside);
    }
    sync_parents(moved.iter().map(PathBuf::as_path))?;
    tracing::warn!(
        %transaction,
        reason,
        "legacy publish debris left a frozen chain that does not verify; the set is re-frozen"
    );
    let event = RecoveryEvent {
        transaction: transaction.to_string(),
        source: "legacy",
        outcome: RecoveryOutcome::Unfrozen,
        files: moved,
        detail: Some(reason.to_string()),
    };
    record(paths, &event)?;
    Ok(event)
}
