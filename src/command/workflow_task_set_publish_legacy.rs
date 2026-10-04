//! Self-healing recovery of the transaction files a publisher without the
//! journal left behind (Issue 271).
//!
//! No backup means staging never completed: discard it without touching live
//! files. Backups prove staging finished, but may also survive an interrupted
//! rollback. Persist verification intent before consuming either kind of
//! evidence, and finish any interrupted move-aside before clearing intent.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract, AcceptanceLock,
    AcceptancePin, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
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

/// Apply the binding legacy-debris decision per transaction, under the lock.
pub(super) fn recover_legacy(
    paths: &JournalPaths,
    scopes: &[(PathBuf, Option<String>)],
    pin_path: &Path,
    tasks_root: &Path,
) -> Result<Vec<RecoveryEvent>> {
    let chain = chain_files(pin_path, tasks_root);
    let marker = pin_path.with_extension("publish-verification");
    let mut events = Vec::new();
    for (transaction, found) in collect(scopes)? {
        if found.backups.is_empty() {
            let event = RecoveryEvent {
                transaction,
                source: "legacy",
                outcome: RecoveryOutcome::Discarded,
                files: found
                    .staged
                    .iter()
                    .map(|(staged, _)| staged.clone())
                    .collect(),
                detail: Some("no backup: staging may be partial; live files untouched".into()),
            };
            record(paths, &event)?;
            for (staged, _) in &found.staged {
                remove_if_present(staged)?;
            }
            sync_parents(found.staged.iter().map(|(staged, _)| staged.as_path()))?;
            events.push(event);
            continue;
        }
        pending(&marker, &transaction)?;
        // A backup target equal to its old bytes, with staging gone, may have
        // been restored by an interrupted rollback. Do not guess new bytes.
        let mut forward = true;
        for backup in &found.backups {
            let target = target_of(backup);
            if !found.staged.iter().any(|(_, path)| path == &target) {
                let live = digest_of(&target)?;
                if live.is_none() || live == digest_of(backup)? {
                    forward = false;
                }
            }
        }
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
        sync_parents(found.staged.iter().map(|(staged, _)| staged.as_path()))?;
        let event = RecoveryEvent {
            transaction,
            source: "legacy",
            outcome: if forward {
                RecoveryOutcome::RolledForward
            } else {
                RecoveryOutcome::VerificationPending
            },
            files,
            detail: Some("durable chain verification pending".into()),
        };
        record(paths, &event)?;
        for backup in &found.backups {
            remove_if_present(backup)?;
        }
        sync_parents(found.backups.iter().map(PathBuf::as_path))?;
        super::journal::crash_point("legacy-backups-removed");
        events.push(event);
    }
    let interrupted = unverified_transaction(&chain)?;
    if marker.exists() || interrupted.is_some() {
        let transaction = match std::fs::read_to_string(&marker) {
            Ok(id) if is_transaction_id(&id) => id,
            Ok(_) => uuid::Uuid::new_v4().simple().to_string(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Inspection files can belong to an already completed recovery.
                // Start a fresh event; only a surviving marker resumes an event.
                uuid::Uuid::new_v4().simple().to_string()
            }
            Err(error) => return Err(error.into()),
        };
        pending(&marker, &transaction)?;
        if let Err(reason) = verify_chain(pin_path, tasks_root) {
            events.push(unfreeze(
                paths,
                pin_path,
                tasks_root,
                &transaction,
                &reason,
            )?);
        } else if !tasks_root.join(ACCEPTANCE_LOCK_FILE).exists() {
            // An older recovery may already have finished every move but
            // recorded no adoption authority. Its retained pin authenticates
            // the data, and the marker keeps this import durable on a crash.
            super::super::recovery_lineage::record_unfrozen_if_missing(
                pin_path,
                tasks_root,
                &transaction,
            )?;
        }
        super::journal::crash_point("legacy-verified");
        remove_if_present(&marker)?;
        super::journal::sync_parent(&marker)?;
    }
    Ok(events)
}

fn pending(marker: &Path, transaction: &str) -> Result<()> {
    let parent = marker
        .parent()
        .ok_or_else(|| anyhow!("verification marker has no parent"))?;
    super::scope::validate_destination(marker, &[(parent.to_path_buf(), None)])?;
    match std::fs::read_to_string(marker) {
        Ok(id) if is_transaction_id(&id) => {
            // Retry a flush a prior attempt may have failed.
            std::fs::OpenOptions::new()
                .write(true)
                .open(marker)?
                .sync_all()?;
        }
        Ok(_) => super::journal::write_durably(marker, transaction.as_bytes())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            super::journal::write_durably(marker, transaction.as_bytes())?;
        }
        Err(error) => return Err(error.into()),
    }
    super::journal::sync_parent(marker)
}

const UNVERIFIED: &str = ".unverified-";

/// Whether any chain file has a moved-aside `.<name>.unverified-<id>` sibling.
fn unverified_transaction(chain: &[PathBuf]) -> Result<Option<String>> {
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
            if let Some(id) = entry.file_name().to_string_lossy().strip_prefix(&prefix)
                && is_transaction_id(id)
            {
                return Ok(Some(id.to_string()));
            }
        }
    }
    Ok(None)
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
pub(crate) fn verify_chain(pin_path: &Path, tasks_root: &Path) -> std::result::Result<(), String> {
    let acceptance_lock = tasks_root.join(ACCEPTANCE_LOCK_FILE);
    let skeleton_lock = tasks_root.join(TASK_SKELETON_LOCK_FILE);
    if !acceptance_lock.exists() {
        let surviving = skeleton_lock.exists()
            || pin_path.exists()
            || sidecar_path(pin_path).is_some_and(|sidecar| sidecar.exists());
        return if surviving {
            Err(format!(
                "frozen chain artifact without {ACCEPTANCE_LOCK_FILE}"
            ))
        } else {
            Ok(())
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
    let contract: AcceptanceContract =
        serde_json::from_value(read_json(&tasks_root.join(ACCEPTANCE_CONTRACT_FILE))?)
            .map_err(|error| format!("invalid acceptance contract: {error}"))?;
    let ids = contract
        .acceptance
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    archon_workflow::task_set_contract::validate_acceptance_bundle(tasks_root, Some(&pin), &ids)
        .map_err(|error| error.to_string())?;
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
    super::super::recovery_lineage::record_unfreeze(pin_path, tasks_root, transaction)?;
    let mut moved = Vec::new();
    let mut targets = vec![
        tasks_root.join(ACCEPTANCE_LOCK_FILE),
        tasks_root.join(TASK_SKELETON_LOCK_FILE),
        pin_path.to_path_buf(),
    ];
    targets.extend(sidecar_path(pin_path));
    for (index, target) in targets
        .iter()
        .enumerate()
        .filter(|(_, target)| target.exists())
    {
        let name = target
            .file_name()
            .ok_or_else(|| anyhow!("{} has no file name", target.display()))?
            .to_string_lossy();
        let aside = target.with_file_name(format!(".{name}{UNVERIFIED}{transaction}"));
        rename(target, &aside)?;
        super::journal::sync_parent(&aside)?;
        super::journal::crash_point(&format!("unfreeze-moved-{index}"));
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
