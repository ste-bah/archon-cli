//! Self-healing recovery of the transaction files a publisher without the
//! journal left behind (Issue 271).
//!
//! Persist a decision before consuming evidence. Discarded chain staging
//! still requires verification: the first freeze may have no backups at all.
//! Only a pending marker authorizes verification and interrupted move-aside.

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
use super::recover::{RecoveryEvent, RecoveryOutcome, record, record_with_authority};
use super::verification::{Decision, Marker};

/// One journal-less transaction's files, by transaction id.
#[derive(Default)]
struct LegacyTransaction {
    staged: Vec<(PathBuf, PathBuf)>,
    backups: Vec<PathBuf>,
}

/// Apply one durable decision per transaction under the publication lock.
pub(super) fn recover_legacy(
    paths: &JournalPaths,
    scopes: &[(PathBuf, Option<String>)],
    pin_path: &Path,
    tasks_root: &Path,
) -> Result<Vec<RecoveryEvent>> {
    let chain = chain_files(pin_path, tasks_root);
    let marker_path = pin_path.with_extension("publish-verification");
    let mut marker = Marker::load(&marker_path)?;
    let mut events = Vec::new();
    for (transaction, found) in collect(scopes)? {
        let decision = marker
            .as_ref()
            .and_then(|marker| marker.decisions.get(&transaction))
            .copied();
        let decision = match decision {
            Some(decision) => decision,
            None if found.backups.is_empty() => Decision::Discard,
            None if found.staged.is_empty() => {
                // No written digest survives. Existing targets may belong to
                // a later publication; only an absent target authorizes restore.
                if found
                    .backups
                    .iter()
                    .any(|backup| !target_of(backup).exists())
                {
                    Decision::Rollback
                } else {
                    Decision::Preserve
                }
            }
            None => {
                let mut rollback = false;
                for backup in &found.backups {
                    let target = target_of(backup);
                    if !found.staged.iter().any(|(_, path)| path == &target)
                        && digest_of(&target)?.is_none()
                    {
                        rollback = true;
                    }
                }
                if rollback {
                    Decision::Rollback
                } else {
                    Decision::Forward
                }
            }
        };
        let verify = found
            .staged
            .iter()
            .any(|(_, target)| chain.contains(target))
            || found
                .backups
                .iter()
                .any(|backup| chain.contains(&target_of(backup)));
        if decision != Decision::Discard || verify {
            let intent = marker.get_or_insert_with(|| {
                let mut intent = Marker::new(&transaction);
                intent.verify_chain = false;
                intent
            });
            if verify && !intent.verify_chain {
                intent.transaction = transaction.clone();
            }
            intent.verify_chain |= verify;
            if !intent.decisions.contains_key(&transaction) {
                let mut written = BTreeMap::new();
                if decision == Decision::Rollback {
                    for backup in &found.backups {
                        let target = target_of(backup);
                        if digest_of(&target)?.is_none() {
                            written.insert(target, None);
                        }
                    }
                }
                if decision == Decision::Preserve {
                    for backup in &found.backups {
                        let target = target_of(backup);
                        written.insert(target.clone(), digest_of(&target)?);
                    }
                }
                intent.written.insert(transaction.clone(), written);
                intent.decisions.insert(transaction.clone(), decision);
            }
            intent.save(&marker_path)?;
        }
        let mut files = Vec::new();
        let mut preserved = Vec::new();
        let mut restored = 0;
        if decision == Decision::Rollback {
            for (index, backup) in found.backups.iter().enumerate() {
                let target = target_of(backup);
                let current = digest_of(&target)?;
                let expected = marker
                    .as_ref()
                    .and_then(|marker| marker.written.get(&transaction))
                    .and_then(|written| written.get(&target));
                // A retry never restores over bytes written after the decision.
                // An older unbound marker can restore only a missing target.
                if expected.map_or(current.is_some(), |expected| *expected != current) {
                    preserved.push(format!(
                        "{} preserved: {}",
                        target.display(),
                        if expected.is_none() {
                            "no transaction-bound file evidence authorizes restoration"
                        } else {
                            "target changed since the transaction's recorded decision"
                        }
                    ));
                    files.push(target);
                    continue;
                }
                rename(backup, &target)?;
                super::journal::sync_parent(&target)?;
                super::journal::crash_point(&format!("legacy-restored-{index}"));
                files.push(target);
                restored += 1;
            }
        }
        for (index, (staged, target)) in found.staged.iter().enumerate() {
            if decision == Decision::Forward {
                rename(staged, target)?;
                files.push(target.clone());
            } else {
                remove_if_present(staged)?;
                files.push(staged.clone());
            }
            super::journal::sync_parent(staged)?;
            super::journal::crash_point(&format!("legacy-renamed-{index}"));
        }
        if decision == Decision::Preserve {
            for backup in &found.backups {
                let target = target_of(backup);
                preserved.push(format!(
                    "{} preserved: no transaction-bound file evidence authorizes restoration",
                    target.display()
                ));
                files.push(target);
            }
        }
        let event = RecoveryEvent {
            transaction,
            source: "legacy",
            outcome: match decision {
                Decision::Discard => RecoveryOutcome::Discarded,
                Decision::Preserve => RecoveryOutcome::Preserved,
                Decision::Forward => RecoveryOutcome::RolledForward,
                Decision::Rollback if restored == 0 && !preserved.is_empty() => {
                    RecoveryOutcome::Preserved
                }
                Decision::Rollback => RecoveryOutcome::RolledBack,
            },
            files,
            detail: Some(if !preserved.is_empty() {
                format!(
                    "restored {restored} target(s); {}{}",
                    preserved.join("; "),
                    if verify {
                        "; durable chain verification pending"
                    } else {
                        ""
                    }
                )
            } else if verify {
                "durable chain verification pending".into()
            } else if decision == Decision::Preserve {
                "live targets preserved; no transaction-bound evidence authorizes their rollback"
                    .into()
            } else {
                "decision bound to this transaction's file evidence".into()
            }),
        };
        super::recover::record_legacy_decision(
            paths,
            &event,
            marker
                .as_ref()
                .and_then(|marker| marker.written.get(&event.transaction)),
        )?;
        for backup in &found.backups {
            remove_if_present(backup)?;
        }
        sync_parents(found.backups.iter().map(PathBuf::as_path))?;
        super::journal::crash_point("legacy-backups-removed");
        events.push(event);
    }
    // Moved-aside inspection files alone grant no new recovery authority.
    if let Some(marker) = marker {
        let interrupted = chain.iter().any(|target| {
            target.file_name().is_some_and(|name| {
                target
                    .with_file_name(format!(
                        ".{}.unverified-{}",
                        name.to_string_lossy(),
                        marker.transaction
                    ))
                    .exists()
            })
        });
        let reason = marker
            .verify_chain
            .then(|| verify_chain(pin_path, tasks_root).err())
            .flatten();
        if marker.verify_chain && (interrupted || reason.is_some()) {
            match unfreeze(
                paths,
                pin_path,
                tasks_root,
                &marker.transaction,
                reason
                    .as_deref()
                    .unwrap_or("finishing interrupted unfreeze"),
            ) {
                Ok(event) => events.push(event),
                Err(error) => {
                    tracing::warn!(%error, "recovery verification deferred; marker retained for retry");
                    let deferred = RecoveryEvent {
                        transaction: marker.transaction.clone(),
                        source: "legacy",
                        outcome: RecoveryOutcome::VerificationPending,
                        files: vec![marker_path.clone()],
                        detail: Some(format!("{error:#}")),
                    };
                    if let Err(error) = record(paths, &deferred) {
                        tracing::warn!(%error, "deferred recovery log will be retried with the marker");
                    }
                    events.push(deferred);
                    return Ok(events);
                }
            }
        }
        super::journal::crash_point("legacy-verified");
        remove_if_present(&marker_path)?;
        super::journal::sync_parent(&marker_path)?;
    }
    if let Err(error) = super::super::recovery_lineage::cleanup_adopted(pin_path, tasks_root) {
        tracing::warn!(%error, "recovery inspection cleanup deferred for retry");
    }
    Ok(events)
}

const UNVERIFIED: &str = ".unverified-";

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
    let authority = super::super::recovery_lineage::authority(pin_path, transaction)?;
    // Bind the run anchors BEFORE the first move too. A crash between moves
    // cannot let a modified pending receipt acquire new run authority on retry.
    let log = match std::fs::read_to_string(&paths.log) {
        Ok(log) => log,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let prior = log
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| {
            event["transaction"] == transaction
                && event.get("authority").is_some_and(|value| !value.is_null())
        });
    match prior {
        Some(event) if event.get("authority") != authority.as_ref() => {
            return Err(anyhow!(
                "pending recovery authority changed after it was logged"
            ));
        }
        Some(_) => {}
        None => record_with_authority(
            paths,
            &RecoveryEvent {
                transaction: transaction.into(),
                source: "legacy",
                outcome: RecoveryOutcome::VerificationPending,
                files: Vec::new(),
                detail: Some("unfreeze authorization captured before moving artifacts".into()),
            },
            authority.clone(),
        )?,
    }
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
    record_with_authority(paths, &event, authority)?;
    Ok(event)
}
