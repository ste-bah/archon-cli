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
use super::worktree_isolation::{IsolationError, run_git};

const MAX_RETRIES: u32 = 600;
const BASE_DELAY_MS: u64 = 100;
const TAIL_BYTES: usize = 4096;

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
                write!(f, "stale baseline for '{item}' at '{path}'")
            }
            Self::VerifyFailed { exit, stderr_tail } => {
                write!(f, "wave verify failed (exit {exit}): {stderr_tail}")
            }
            Self::PersistFailed { source } => write!(f, "persist failed: {source}"),
            Self::Isolation(e) => write!(f, "isolation error: {e}"),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyResumeStatus {
    NotPersisted,
    Applied,
    IdempotentNoop,
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
    with_repo_lock_tuned(canonical_root, MAX_RETRIES, BASE_DELAY_MS, f)
}

/// Lock path: `<canonical_root>/.archon/workflows/write-locks/<blake3>.lock`.
pub fn lock_path_for(canonical_root: &Path) -> PathBuf {
    let hex = blake3::hash(canonical_root.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    canonical_root
        .join(".archon")
        .join("workflows")
        .join("write-locks")
        .join(format!("{hex}.lock"))
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
    };
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
    // VAL-WC-004 stale recheck — only files this item INTENDS to mutate.
    if let Some(path) = stale_target(canonical_root, m, pre_hashes_by_item) {
        updated.status = ManifestStatus::Failed {
            reason: format!("stale baseline at {path}"),
        };
        rec.items_failed
            .push((m.item_id.clone(), format!("StaleBaseline at {path}")));
        persist_status(run_root, run_id, stage_id, &m.item_id, &updated)?;
        return Ok(());
    }
    let patch_str = m.patch_path.to_string_lossy().into_owned();
    match run_git(
        &["apply", "--3way", "--whitespace=nowarn", &patch_str],
        canonical_root,
    ) {
        Ok(_) => {
            updated.post_hashes = hash_targets(canonical_root, &m.declared_target_files);
            updated.status = ManifestStatus::Applied;
            persist_status(run_root, run_id, stage_id, &m.item_id, &updated)?;
            rec.items_applied.push(m.item_id.clone());
            Ok(())
        }
        Err(IsolationError::ProcessFailed { stderr }) => {
            cleanup_conflict_markers(canonical_root, &m.changed_files);
            let reason = stderr.lines().next().unwrap_or("").to_string();
            updated.status = ManifestStatus::Failed {
                reason: reason.clone(),
            };
            rec.items_failed
                .push((m.item_id.clone(), format!("PatchApplyConflict: {reason}")));
            persist_status(run_root, run_id, stage_id, &m.item_id, &updated)?;
            Ok(())
        }
        Err(e) => Err(map_apply_error_with_item(&m.item_id, e)),
    }
}

fn stale_target(
    canonical_root: &Path,
    m: &PatchManifest,
    pre_hashes_by_item: &BTreeMap<ItemId, BTreeMap<String, String>>,
) -> Option<String> {
    let expected = pre_hashes_by_item.get(&m.item_id)?;
    m.declared_target_files
        .iter()
        .filter(|t| m.changed_files.iter().any(|c| c == *t))
        .find(|t| {
            let now = hash_file(&canonical_root.join(t));
            expected
                .get(t.as_str())
                .is_some_and(|exp| now.as_deref() != Some(exp))
        })
        .cloned()
}

fn cleanup_conflict_markers(canonical_root: &Path, changed: &[String]) {
    if changed.is_empty() {
        return;
    }
    let mut args: Vec<&str> = vec!["checkout", "HEAD", "--"];
    args.extend(changed.iter().map(String::as_str));
    let _ = run_git(&args, canonical_root);
}

fn hash_file(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
}

fn hash_targets(canonical_root: &Path, targets: &[String]) -> BTreeMap<String, String> {
    targets
        .iter()
        .map(|t| {
            let h = hash_file(&canonical_root.join(t)).unwrap_or_else(|| "deleted".to_string());
            (t.clone(), h)
        })
        .collect()
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

/// MUST be invoked from inside the SAME `with_repo_lock` closure that called
/// apply_wave. The caller sequences both inside ONE closure so the lock is
/// contiguously held.
pub fn run_wave_verify(
    canonical_root: &Path,
    verify_command: Option<&str>,
    wave_id: WaveId,
    run_root: &Path,
    stage_id: &str,
) -> Result<VerifyResult, ApplyError> {
    let Some(cmd) = verify_command else {
        let result = VerifyResult {
            exit: 0,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            duration_ms: 0,
        };
        persist_verify(run_root, stage_id, wave_id, &result)?;
        return Ok(result);
    };
    let start = SystemTime::now();
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(canonical_root)
        .output()
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                ApplyError::GitMissing
            } else {
                ApplyError::LockIo(source)
            }
        })?;
    let duration_ms = start.elapsed().map(|d| d.as_millis() as u64).unwrap_or(0);
    let result = VerifyResult {
        exit: output.status.code().unwrap_or(-1),
        stdout_tail: utf8_safe_tail(&output.stdout, TAIL_BYTES),
        stderr_tail: utf8_safe_tail(&output.stderr, TAIL_BYTES),
        duration_ms,
    };
    persist_verify(run_root, stage_id, wave_id, &result)?;
    Ok(result)
}

/// Resume granularity for AC-WC-010: load the persisted manifest status.
pub fn resume_status(item_id: &ItemId, run_root: &Path, stage_id: &str) -> ApplyResumeStatus {
    let path = run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id)
        .join("manifests")
        .join(format!("{item_id}.json"));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ApplyResumeStatus::NotPersisted;
    };
    let Ok(manifest) = serde_json::from_str::<PatchManifest>(&text) else {
        return ApplyResumeStatus::NotPersisted;
    };
    match manifest.status {
        ManifestStatus::Applied => ApplyResumeStatus::Applied,
        ManifestStatus::IdempotentNoop => ApplyResumeStatus::IdempotentNoop,
        ManifestStatus::Conflicted => ApplyResumeStatus::Conflicted,
        ManifestStatus::PendingApply => ApplyResumeStatus::PendingApply,
        ManifestStatus::Failed { reason } => ApplyResumeStatus::Failed(reason),
    }
}

fn persist_record(
    run_root: &Path,
    stage_id: &str,
    wave_id: WaveId,
    rec: &ApplyRecord,
) -> Result<(), ApplyError> {
    let dir = stage_dir(run_root, stage_id).join("apply");
    let json = serde_json::to_vec_pretty(rec).map_err(|e| ApplyError::PersistFailed {
        source: std::io::Error::other(e),
    })?;
    write_atomic(&dir.join(format!("{wave_id}.json")), &json)
}

fn persist_verify(
    run_root: &Path,
    stage_id: &str,
    wave_id: WaveId,
    result: &VerifyResult,
) -> Result<(), ApplyError> {
    let dir = stage_dir(run_root, stage_id).join("tests");
    let json = serde_json::to_vec_pretty(result).map_err(|e| ApplyError::PersistFailed {
        source: std::io::Error::other(e),
    })?;
    write_atomic(&dir.join(format!("{wave_id}.json")), &json)
}

fn stage_dir(run_root: &Path, stage_id: &str) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ApplyError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ApplyError::PersistFailed { source })?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|source| ApplyError::PersistFailed { source })?;
    std::fs::rename(&tmp, path).map_err(|source| ApplyError::PersistFailed { source })?;
    Ok(())
}

fn persist_io(e: super::patch_manifest::PatchError) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

fn with_repo_lock_tuned<R, F>(
    canonical_root: &Path,
    max_retries: u32,
    base_delay_ms: u64,
    f: F,
) -> Result<R, ApplyError>
where
    F: FnOnce() -> Result<R, ApplyError>,
{
    let lock_path = lock_path_for(canonical_root);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).map_err(ApplyError::LockIo)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(ApplyError::LockIo)?;
    let mut lock = fd_lock::RwLock::new(file);
    for attempt in 0..max_retries {
        match lock.try_write() {
            Ok(_guard) => return f(),
            Err(_) => std::thread::sleep(Duration::from_millis(
                base_delay_ms + (u64::from(attempt) * 7 % 30),
            )),
        }
    }
    Err(ApplyError::LockTimeout {
        lock_path,
        waited: Duration::from_millis(u64::from(max_retries) * base_delay_ms),
    })
}

/// Last `max` bytes decoded at a valid UTF-8 boundary (never invalid bytes).
fn utf8_safe_tail(bytes: &[u8], max: usize) -> String {
    let start = bytes.len().saturating_sub(max);
    match std::str::from_utf8(&bytes[start..]) {
        Ok(valid) => valid.to_string(),
        Err(e) => {
            let boundary = start + e.valid_up_to();
            String::from_utf8_lossy(&bytes[boundary..]).into_owned()
        }
    }
}

#[cfg(test)]
#[path = "patch_apply_tests.rs"]
mod tests;
