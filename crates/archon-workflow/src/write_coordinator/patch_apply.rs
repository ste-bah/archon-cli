//! TASK-WC-006 — Cross-run canonical patch apply behind a repository write lock.
//!
//! Applies validated PatchManifests serially (deterministic by item id) to the
//! canonical repo under a cross-process advisory lock, re-checks declared-target
//! baselines before each apply, and runs the wave verify command. The lock is
//! held via the `with_repo_lock` closure for the entire apply+verify sequence.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use super::ItemId;
use super::WaveId;
use super::patch_manifest::{ManifestStatus, PatchManifest, persist_manifest_status_update};
use super::worktree_isolation::IsolationError;

mod applied_judgement;
mod apply_git;
mod file_backup;
mod landing_failure;
mod lock;
mod materialize;
mod materialize_ledger;
mod materialize_scope;
mod persist;
mod project_inputs_apply;
mod project_inputs_ledger;
pub(crate) use project_inputs_ledger::{ProjectInputLanding, run_project_input_landings};
mod refused_revert;
pub(crate) use refused_revert::{DataRevert, revert_copy, revert_input};
mod resume;
mod targets;
pub(crate) use targets::hash_file;
use targets::{hash_targets, landed_paths, stale_target, touched_files};
mod verify;
mod wave_commit;
use landing_failure::{attention_prefixed, fail_materialization, flag_attention};
pub use lock::lock_path_for;
use lock::with_repo_lock_default;
#[cfg(test)]
use lock::with_repo_lock_tuned;
pub(crate) use materialize::{run_materializations, universe_deliverables};
#[cfg(test)]
pub(crate) use materialize_ledger::append as append_materializations;
pub(crate) use materialize_scope::{ENGINE_LOADED, destination_baselines};
#[cfg(test)]
use persist::utf8_safe_tail;
use persist::{persist_io, persist_record};
pub use resume::resume_status;
pub use verify::run_wave_verify;

#[derive(Debug)]
pub enum ApplyError {
    LockTimeout {
        lock_path: PathBuf,
        waited: Duration,
    },
    LockIo(std::io::Error),
    GitMissing,
    ConflictGraphViolation {
        conflicting_paths: Vec<String>,
    },
    PatchApplyConflict {
        item: ItemId,
        stderr: String,
    },
    StaleBaseline {
        item: ItemId,
        path: String,
    },
    VerifyFailed {
        exit: i32,
        stderr_tail: String,
    },
    PersistFailed {
        source: std::io::Error,
    },
    Isolation(IsolationError),
    WaveCommitFailed {
        stderr: String,
    },
    UnknownItem(ItemId),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockTimeout { lock_path, waited } => {
                write!(
                    f,
                    "repo write lock timeout after {waited:?} at {lock_path:?}"
                )
            }
            Self::LockIo(e) => write!(f, "lock io error: {e}"),
            Self::GitMissing => write!(f, "git executable not found"),
            Self::ConflictGraphViolation { conflicting_paths } => {
                write!(
                    f,
                    "conflict graph violation: overlapping paths {conflicting_paths:?}"
                )
            }
            Self::PatchApplyConflict { item, stderr } => {
                write!(f, "patch apply conflict for '{item}': {stderr}")
            }
            Self::StaleBaseline { item, path } => {
                // Carry the remedy: the patch was computed against a version of
                // this file that has since changed, so re-applying it would
                // clobber the newer content. Re-read and regenerate.
                write!(
                    f,
                    "stale baseline for '{item}' at '{path}': this file changed after your patch was computed, so the patch was NOT applied. Re-read '{path}' as it is NOW and regenerate your change against the current contents — do not resubmit the same patch"
                )
            }
            Self::VerifyFailed { exit, stderr_tail } => {
                write!(f, "wave verify failed (exit {exit}): {stderr_tail}")
            }
            Self::PersistFailed { source } => write!(f, "persist failed: {source}"),
            Self::Isolation(e) => write!(f, "isolation error: {e}"),
            Self::WaveCommitFailed { stderr } => {
                write!(f, "wave output commit failed: {stderr}")
            }
            Self::UnknownItem(item) => write!(f, "unknown item '{item}'"),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<IsolationError> for ApplyError {
    /// Top-level propagation OUTSIDE per-item loops (no item context). Never
    /// produces PatchApplyConflict — that variant requires an item id.
    fn from(e: IsolationError) -> Self {
        match e {
            IsolationError::GitMissing => ApplyError::GitMissing,
            other => ApplyError::Isolation(other),
        }
    }
}

/// Inside a per-item loop where the item id is in scope, prefer this over the
/// bare `From` impl so ProcessFailed carries the item id.
pub fn map_apply_error_with_item(item: &ItemId, e: IsolationError) -> ApplyError {
    match e {
        IsolationError::GitMissing => ApplyError::GitMissing,
        IsolationError::ProcessFailed { stderr } => ApplyError::PatchApplyConflict {
            item: item.clone(),
            stderr,
        },
        other => ApplyError::Isolation(other),
    }
}

mod system_time_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let millis = t
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        s.serialize_u64(millis)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let millis = u64::deserialize(d)?;
        Ok(UNIX_EPOCH + Duration::from_millis(millis))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyResult {
    pub exit: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyRecord {
    pub wave_id: WaveId,
    #[serde(with = "system_time_millis")]
    pub started_at: SystemTime,
    #[serde(with = "system_time_millis")]
    pub completed_at: SystemTime,
    pub items_applied: Vec<ItemId>,
    pub items_failed: Vec<(ItemId, String)>,
    pub verify_result: Option<VerifyResult>,
    /// Items whose project-input changes were refused (Batch E): what their
    /// patch carried stands; their project data did not land.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub project_input_refusals: Vec<(ItemId, String)>,
    /// Batch K: repository test material refused as project data, one HIGH
    /// finding each (`fixture_provenance`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixture_landings: Vec<(ItemId, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyResumeStatus {
    NotPersisted,
    Applied,
    IdempotentNoop,
    SkippedIgnored,
    Failed(String),
    Conflicted,
    PendingApply,
}

/// Cross-process advisory write lock for one canonical repo. Holds the lock for
/// the entire closure. Caller MUST sequence apply_wave + run_wave_verify inside
/// ONE invocation so the lock is held contiguously.
pub fn with_repo_lock<R, F>(canonical_root: &Path, f: F) -> Result<R, ApplyError>
where
    F: FnOnce() -> Result<R, ApplyError>,
{
    with_repo_lock_default(canonical_root, f)
}

/// MUST be invoked from inside `with_repo_lock`. Sequence apply_wave +
/// run_wave_verify within ONE closure so the lock is held contiguously.
pub fn apply_wave(
    canonical_root: &Path,
    manifests: &[PatchManifest],
    pre_hashes_by_item: &BTreeMap<ItemId, BTreeMap<String, String>>,
    wave_id: WaveId,
    run_root: &Path,
    run_id: &str,
    stage_id: &str,
) -> Result<ApplyRecord, ApplyError> {
    assert_no_path_overlap(manifests)?;
    let mut order: Vec<usize> = (0..manifests.len()).collect();
    order.sort_by_key(|&i| manifests[i].item_id.clone());

    let mut rec = ApplyRecord {
        wave_id,
        started_at: SystemTime::now(),
        completed_at: SystemTime::now(),
        items_applied: Vec::new(),
        items_failed: Vec::new(),
        verify_result: None,
        project_input_refusals: Vec::new(),
        fixture_landings: Vec::new(),
    };
    // Batch G: every project write below is the host's own (`input_tripwire`).
    let _section = super::input_tripwire::landing_section(canonical_root, manifests);
    for &i in &order {
        apply_one(
            canonical_root,
            &manifests[i],
            pre_hashes_by_item,
            run_root,
            run_id,
            stage_id,
            &mut rec,
        )?;
    }
    // Only what landed is committed: a refused item's tree was put back, and
    // a path it created no longer exists for `git add` to name (Batch K).
    let landed: Vec<PatchManifest> = (manifests.iter())
        .filter(|m| !rec.items_failed.iter().any(|(item, _)| item == &m.item_id))
        .cloned()
        .collect();
    wave_commit::commit_wave_outputs(canonical_root, &landed, run_id, stage_id, wave_id)?;
    rec.completed_at = SystemTime::now();
    persist_record(run_root, stage_id, wave_id, &rec)?;
    Ok(rec)
}

/// Conflict-graph-violation guard: no path may appear across two manifests.
fn assert_no_path_overlap(manifests: &[PatchManifest]) -> Result<(), ApplyError> {
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    let mut conflicts = Vec::new();
    for m in manifests {
        for path in &m.declared_target_files {
            if seen.insert(path.clone(), ()).is_some() {
                conflicts.push(path.clone());
            }
        }
    }
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(ApplyError::ConflictGraphViolation {
            conflicting_paths: conflicts,
        })
    }
}

fn apply_one(
    canonical_root: &Path,
    m: &PatchManifest,
    pre_hashes_by_item: &BTreeMap<ItemId, BTreeMap<String, String>>,
    run_root: &Path,
    run_id: &str,
    stage_id: &str,
    rec: &mut ApplyRecord,
) -> Result<(), ApplyError> {
    let mut updated = m.clone();
    match super::patch_sidecar::archive(&m.patch_path, run_root, stage_id, &m.item_id) {
        Ok(archived) => updated.skipped_ignored.extend(archived),
        Err(err) => {
            updated.status = ManifestStatus::Failed {
                reason: format!("ignored artifact retention failed: {err}"),
            };
            rec.items_failed.push((m.item_id.clone(), err.to_string()));
            persist_status(run_root, run_id, stage_id, &m.item_id, &updated)?;
            return Ok(());
        }
    }
    if matches!(
        m.status,
        ManifestStatus::IdempotentNoop | ManifestStatus::SkippedIgnored
    ) {
        if !updated.skipped_ignored.is_empty() {
            updated.status = ManifestStatus::SkippedIgnored;
        }
        // Issue-113: an ignored project artifact lands where it is verified.
        let landed = materialize::materialize(run_root, canonical_root, &mut updated)
            .and_then(|undo| materialize::record(run_root, &mut updated, undo));
        if let Err(failure) = landed {
            return fail_materialization(run_root, run_id, stage_id, updated, rec, failure);
        }
        project_inputs_apply::land(run_root, canonical_root, &updated, false, rec);
        persist_status(run_root, run_id, stage_id, &m.item_id, &updated)?;
        return Ok(());
    }
    // VAL-WC-004 stale recheck — only files this item INTENDS to mutate.
    if let Some(path) = stale_target(canonical_root, m, pre_hashes_by_item) {
        updated.status = ManifestStatus::Failed {
            reason: format!(
                "stale baseline at {path}: the file changed after this patch was computed and was NOT modified; re-read {path} as it is now and regenerate the change against current contents"
            ),
        };
        rec.items_failed
            .push((m.item_id.clone(), format!("StaleBaseline at {path}")));
        persist_status(run_root, run_id, stage_id, &m.item_id, &updated)?;
        return Ok(());
    }
    let backup = file_backup::FileBackups::capture(canonical_root, &touched_files(m))
        .map_err(ApplyError::LockIo)?;
    // Before the patch, so a copy that fails leaves the tree untouched and a
    // patch that fails puts every copy back.
    let undo = match materialize::materialize(run_root, canonical_root, &mut updated) {
        Ok(undo) => undo,
        Err(failure) => {
            return fail_materialization(run_root, run_id, stage_id, updated, rec, failure);
        }
    };
    let patch_str = m.patch_path.to_string_lossy().into_owned();
    let applied = apply_git::apply_patch(canonical_root, &patch_str, &m.changed_files);
    let undo = match &applied {
        Ok(_) => {
            // Batch K (I1): the applied tree is judged before it is recorded.
            let recorded =
                match applied_judgement::judge_applied(run_root, canonical_root, &updated) {
                    Some(failure) => Err(materialize::abandon(&mut updated, undo, failure)),
                    None => materialize::record(run_root, &mut updated, undo),
                };
            if let Err(failure) = recorded {
                // The copies are undone (or flagged); the patch must go too.
                // A `--3way` apply also staged it: the index goes back too.
                let restored = backup.restore(canonical_root).map_err(|e| e.to_string());
                let restored = restored.and_then(|()| apply_git::unstage(canonical_root, m));
                if let Err(error) = restored {
                    flag_attention(&mut updated, &format!("tracked restore failed: {error}"));
                }
                return fail_materialization(run_root, run_id, stage_id, updated, rec, failure);
            }
            None
        }
        Err(_) => Some(undo),
    };
    if let Some(undo) = undo {
        match undo.restore() {
            Ok(()) => updated.materialized.clear(),
            Err(error) => flag_attention(
                &mut updated,
                &format!(
                    "the patch did not apply and its materialized copies could not be undone: {error}"
                ),
            ),
        }
    }
    match applied {
        Ok(_) => {
            updated.post_hashes = hash_targets(canonical_root, &landed_paths(m));
            updated.status = ManifestStatus::Applied;
            project_inputs_apply::land(run_root, canonical_root, &updated, true, rec);
            persist_status(run_root, run_id, stage_id, &m.item_id, &updated)?;
            rec.items_applied.push(m.item_id.clone());
            Ok(())
        }
        Err(IsolationError::ProcessFailed { stderr }) => {
            let reason = stderr.lines().next().unwrap_or("").to_string();
            let restore_error = backup.restore(canonical_root).err();
            let reason = match restore_error {
                Some(err) => {
                    flag_attention(&mut updated, &format!("tracked restore failed: {err}"));
                    format!("{reason}; restore failed: {err}")
                }
                None => reason,
            };
            updated.status = ManifestStatus::Failed {
                reason: reason.clone(),
            };
            let reason = attention_prefixed(&updated, &format!("PatchApplyConflict: {reason}"));
            rec.items_failed.push((m.item_id.clone(), reason));
            persist_status(run_root, run_id, stage_id, &m.item_id, &updated)?;
            Ok(())
        }
        Err(e) => {
            if updated.needs_attention.is_some() {
                updated.status = ManifestStatus::Failed {
                    reason: format!("patch apply error: {e}"),
                };
                persist_status(run_root, run_id, stage_id, &m.item_id, &updated)?;
            }
            Err(map_apply_error_with_item(&m.item_id, e))
        }
    }
}

fn persist_status(
    run_root: &Path,
    run_id: &str,
    stage_id: &str,
    item_id: &ItemId,
    manifest: &PatchManifest,
) -> Result<(), ApplyError> {
    persist_manifest_status_update(run_root, run_id, stage_id, item_id, manifest).map_err(|e| {
        ApplyError::PersistFailed {
            source: persist_io(e),
        }
    })
}

#[cfg(test)]
#[path = "patch_apply_dirty_tests.rs"]
mod dirty_tests;

#[cfg(test)]
#[path = "patch_apply_landed_tests.rs"]
mod landed_tests;
#[cfg(test)]
#[path = "patch_apply_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "patch_apply_wave_commit_tests.rs"]
mod wave_commit_tests;
