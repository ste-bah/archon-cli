//! TASK-WC-003 — Per-item Git worktree isolation (PRD-012 §7.2, §7.3, §9, §14).
//!
//! Owns the filesystem side of write coordination: reproduce the canonical
//! dirty state inside a private worktree, seal a detached baseline commit, and
//! verify the canonical declared-target hashes did not change behind our back.

mod git;

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use thiserror::Error;

use super::WriteCoordinatorConfig;
use super::write_plan::{NormalizedPath, WritePlan};

pub(crate) use git::{run_git, run_git_with_stdin};

/// File identity used for canonical-mutation detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub blake3_hex: String,
    pub mode: u32,
    pub symlink_target: Option<String>,
    pub exists: bool,
}

/// Captured canonical state needed to reproduce the item workspace.
pub struct CanonicalBaseline {
    pub tracked_diff_binary: Vec<u8>,
    pub untracked_files: BTreeMap<String, Vec<u8>>,
    pub declared_target_meta: BTreeMap<String, FileMeta>,
    pub verify_input_meta: BTreeMap<String, FileMeta>,
}

/// Custom Debug: never print untracked file bytes (may hold secrets).
impl fmt::Debug for CanonicalBaseline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let untracked: BTreeMap<&String, usize> = self
            .untracked_files
            .iter()
            .map(|(path, bytes)| (path, bytes.len()))
            .collect();
        f.debug_struct("CanonicalBaseline")
            .field("tracked_diff_binary_len", &self.tracked_diff_binary.len())
            .field("untracked_files(path=>len)", &untracked)
            .field("declared_target_meta", &self.declared_target_meta)
            .field("verify_input_meta", &self.verify_input_meta)
            .finish()
    }
}

/// A created per-item worktree plus its sealed baseline commit.
#[derive(Debug, Clone)]
pub struct ItemWorkspace {
    pub plan: WritePlan,
    pub baseline_commit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Error)]
pub enum IsolationError {
    #[error("git executable not found on PATH")]
    GitMissing,
    #[error("git worktree add failed: {0}")]
    WorktreeAddFailed(String),
    #[error("git apply of tracked overlay failed: {0}")]
    ApplyFailed(String),
    #[error("failed to copy file into isolated workspace: {0}")]
    CopyFailed(String),
    #[error("baseline commit failed: {0}")]
    BaselineCommitFailed(String),
    #[error("canonical repository mutated under coordination at '{path}'")]
    CanonicalMutation { path: String },
    #[error("content hash mismatch at '{path}'")]
    HashMismatch { path: String },
    #[error("file '{path}' is {size} bytes, exceeds max_file_bytes")]
    FileTooLarge { path: String, size: u64 },
    #[error("git process failed: {stderr}")]
    ProcessFailed { stderr: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Capture the canonical dirty state (PRD-012 §9). Reads bytes ONLY for
/// declared targets ∪ verify_inputs, and rejects oversize files at capture time.
pub fn capture_canonical_baseline(
    canonical_root: &Path,
    plan: &WritePlan,
    verify_inputs: &[NormalizedPath],
    cfg: &WriteCoordinatorConfig,
) -> Result<CanonicalBaseline, IsolationError> {
    let tracked_diff_binary = run_git(&["diff", "--binary", "HEAD", "--"], canonical_root)?.stdout;

    let declared: Vec<String> = plan.target_files.iter().map(NormalizedPath::as_str).collect();
    let verify: Vec<String> = verify_inputs.iter().map(NormalizedPath::as_str).collect();
    let mut wanted: std::collections::BTreeSet<&str> =
        declared.iter().map(String::as_str).collect();
    wanted.extend(verify.iter().map(String::as_str));

    let untracked_files =
        capture_untracked(canonical_root, &wanted, cfg.max_file_bytes)?;

    let mut declared_target_meta = BTreeMap::new();
    for path in &declared {
        declared_target_meta.insert(path.clone(), file_meta(&canonical_root.join(path))?);
    }
    let mut verify_input_meta = BTreeMap::new();
    for path in &verify {
        verify_input_meta.insert(path.clone(), file_meta(&canonical_root.join(path))?);
    }
    Ok(CanonicalBaseline {
        tracked_diff_binary,
        untracked_files,
        declared_target_meta,
        verify_input_meta,
    })
}

/// Enumerate untracked paths, then read bytes ONLY for wanted (declared ∪
/// verify) paths, rejecting oversize files before loading them.
fn capture_untracked(
    canonical_root: &Path,
    wanted: &std::collections::BTreeSet<&str>,
    max_file_bytes: u64,
) -> Result<BTreeMap<String, Vec<u8>>, IsolationError> {
    let listing =
        run_git(&["ls-files", "--others", "--exclude-standard", "-z"], canonical_root)?.stdout;
    let mut untracked = BTreeMap::new();
    for raw in listing.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let path = String::from_utf8_lossy(raw).into_owned();
        if !wanted.contains(path.as_str()) {
            continue;
        }
        let abs = canonical_root.join(&path);
        let size = std::fs::metadata(&abs)?.len();
        if size > max_file_bytes {
            return Err(IsolationError::FileTooLarge { path, size });
        }
        untracked.insert(path, std::fs::read(&abs)?);
    }
    Ok(untracked)
}

/// Create the per-item worktree, overlay the dirty state, seal the baseline.
pub fn create_item_workspace(
    canonical_root: &Path,
    plan: &WritePlan,
    baseline: &CanonicalBaseline,
) -> Result<ItemWorkspace, IsolationError> {
    if let Some(parent) = plan.isolated_root.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let isolated_str = plan.isolated_root.to_string_lossy().into_owned();
    run_git(&["worktree", "add", "--detach", &isolated_str, "HEAD"], canonical_root)
        .map_err(|err| IsolationError::WorktreeAddFailed(err.to_string()))?;
    let isolated = plan.isolated_root.as_path();

    if !baseline.tracked_diff_binary.is_empty() {
        run_git_with_stdin(
            &["apply", "--whitespace=nowarn", "--3way", "-"],
            isolated,
            &baseline.tracked_diff_binary,
        )
        .map_err(|err| IsolationError::ApplyFailed(err.to_string()))?;
    }
    for (path, bytes) in &baseline.untracked_files {
        let mode = baseline.declared_target_meta.get(path).map(|m| m.mode);
        write_with_mode(&isolated.join(path), bytes, mode)?;
    }

    run_git(&["add", "-A"], isolated)?;
    run_git(
        &[
            "-c",
            "user.name=archon-workflow",
            "-c",
            "user.email=archon-workflow@local",
            "commit",
            "--no-gpg-sign",
            "--no-verify",
            "--allow-empty",
            "-m",
            &baseline_message(plan),
        ],
        isolated,
    )
    .map_err(|err| IsolationError::BaselineCommitFailed(err.to_string()))?;

    let baseline_commit = run_git(&["rev-parse", "HEAD"], isolated)?;
    let baseline_commit = String::from_utf8_lossy(&baseline_commit.stdout).trim().to_string();
    Ok(ItemWorkspace {
        plan: plan.clone(),
        baseline_commit,
    })
}

fn baseline_message(plan: &WritePlan) -> String {
    format!("archon workflow baseline {} {}", plan.run_id, plan.item_id)
}

fn write_with_mode(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<(), IsolationError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| IsolationError::CopyFailed(err.to_string()))?;
    }
    std::fs::write(path, bytes).map_err(|err| IsolationError::CopyFailed(err.to_string()))?;
    set_mode(path, mode)?;
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>) -> Result<(), IsolationError> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode.filter(|m| *m != 0) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|err| IsolationError::CopyFailed(err.to_string()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: Option<u32>) -> Result<(), IsolationError> {
    Ok(())
}

/// Verify declared targets + verify_inputs in canonical are unchanged (§7.3).
pub fn detect_canonical_mutation(
    canonical_root: &Path,
    baseline: &CanonicalBaseline,
    declared_targets: &[NormalizedPath],
    verify_inputs: &[NormalizedPath],
) -> Result<(), IsolationError> {
    for path in declared_targets.iter().chain(verify_inputs.iter()) {
        let key = path.as_str();
        let Some(expected) = baseline
            .declared_target_meta
            .get(&key)
            .or_else(|| baseline.verify_input_meta.get(&key))
        else {
            continue;
        };
        let now = file_meta(&canonical_root.join(&key))?;
        if &now != expected {
            return Err(IsolationError::CanonicalMutation { path: key });
        }
    }
    Ok(())
}

/// Remove or retain the worktree per the retention policy (§14).
pub fn cleanup_workspace(
    canonical_root: &Path,
    isolated_root: &Path,
    status: WorkspaceStatus,
    cfg: &WriteCoordinatorConfig,
) -> Result<(), IsolationError> {
    let should_remove = match status {
        WorkspaceStatus::Succeeded => !cfg.retain_success_worktrees,
        WorkspaceStatus::Failed => !cfg.retain_failed_worktrees,
    };
    if should_remove {
        let isolated_str = isolated_root.to_string_lossy().into_owned();
        run_git(&["worktree", "remove", "--force", &isolated_str], canonical_root)?;
        run_git(&["worktree", "prune"], canonical_root)?;
    } else if status == WorkspaceStatus::Succeeded {
        // Retained-success: still clear stale admin entries; never touches a
        // worktree whose directory still exists, so retained dirs survive.
        run_git(&["worktree", "prune"], canonical_root)?;
    }
    Ok(())
}

/// File identity for mutation detection. A missing path yields exists=false.
fn file_meta(path: &Path) -> Result<FileMeta, IsolationError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileMeta {
                blake3_hex: String::new(),
                mode: 0,
                symlink_target: None,
                exists: false,
            });
        }
        Err(err) => return Err(IsolationError::Io(err)),
    };
    let symlink_target = if meta.file_type().is_symlink() {
        Some(std::fs::read_link(path)?.to_string_lossy().into_owned())
    } else {
        None
    };
    let blake3_hex = if meta.file_type().is_file() {
        blake3::hash(&std::fs::read(path)?).to_hex().to_string()
    } else {
        String::new()
    };
    Ok(FileMeta {
        blake3_hex,
        mode: file_mode(&meta),
        symlink_target,
        exists: true,
    })
}

#[cfg(unix)]
fn file_mode(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

#[cfg(not(unix))]
fn file_mode(_meta: &std::fs::Metadata) -> u32 {
    0
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
