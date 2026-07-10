//! TASK-WC-005 — Patch capture from the isolated workspace + validation manifest.
//!
//! After the agent finishes, capture its patch with a SINGLE `git diff --binary
//! HEAD -- <targets>` (already combines staged + unstaged), validate it against
//! the declared contract before it touches canonical, and persist the durable
//! manifest + patch evidence.

mod code_hygiene;
mod secret_scan;
mod target_hashes;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::ItemId;
use super::worktree_isolation::{CanonicalBaseline, ItemWorkspace, run_git};
use super::write_plan::{NormalizedPath, WritePlan, normalize_target};
use crate::write_coordinator::WriteCoordinatorConfig;

use self::target_hashes::target_hashes;

pub const PATCH_MANIFEST_SCHEMA: &str = "archon.workflow.patch_manifest.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ManifestStatus {
    PendingApply,
    Applied,
    Failed { reason: String },
    Conflicted,
    IdempotentNoop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchManifest {
    pub schema: String,
    pub run_id: String,
    pub stage_id: String,
    pub item_id: ItemId,
    pub baseline_commit: String,
    pub patch_path: PathBuf,
    pub declared_target_files: Vec<String>,
    pub changed_files: Vec<String>,
    pub created_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub pre_hashes: BTreeMap<String, String>,
    pub post_hashes: BTreeMap<String, String>,
    pub verify_command: Option<String>,
    pub agent_artifact_path: Option<String>,
    pub status: ManifestStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedPatch {
    pub patch_bytes: Vec<u8>,
    pub changed_files: Vec<String>,
    pub created_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub pre_hashes: BTreeMap<String, String>,
    pub post_hashes: BTreeMap<String, String>,
    pub baseline_commit: String,
}

#[derive(Debug, Error)]
pub enum PatchError {
    #[error("git diff failed: {stderr}")]
    GitDiffFailed { stderr: String },
    #[error("patch writes undeclared path '{path}'")]
    UndeclaredWrite { path: String },
    #[error(
        "dynamic target adoption limit reached at undeclared path '{path}' after {adopted} adopted target(s), max {max}"
    )]
    DynamicTargetAdoptionLimit {
        path: String,
        adopted: usize,
        max: usize,
    },
    #[error("patch path '{path}' escapes repository via symlink")]
    SymlinkEscape { path: String },
    #[error("file '{path}' is {size} bytes, exceeds max {max}")]
    FileTooLarge { path: String, size: u64, max: u64 },
    #[error("source file '{path}' is {lines} lines, exceeds max {max}")]
    FileTooManyLines { path: String, lines: u32, max: u32 },
    #[error("function '{function}' in '{path}' has complexity {complexity}, exceeds max {max}")]
    FunctionTooComplex {
        path: String,
        function: String,
        complexity: u32,
        max: u32,
    },
    #[error("patch is {size} bytes, exceeds max {max}")]
    PatchTooLarge { size: u64, max: u64 },
    #[error("secret '{rule}' detected in patch: {line_preview}")]
    SecretDetected {
        rule: &'static str,
        line_preview: String,
    },
    #[error("patch is empty and item did not declare idempotent_noop")]
    EmptyPatch,
    #[error("agent output not usable: {detail}")]
    OutputNotUsable { detail: String },
    #[error("verification blocked after patch: {detail}")]
    VerificationBlockedAfterPatch { detail: String },
    #[error("failed to persist manifest: {source}")]
    PersistFailed {
        #[source]
        source: std::io::Error,
    },
}

/// Capture the agent's patch from the isolated workspace (PRD §11).
pub fn capture_patch(
    workspace: &ItemWorkspace,
    declared_targets: &[NormalizedPath],
    baseline: &CanonicalBaseline,
) -> Result<CapturedPatch, PatchError> {
    let isolated = workspace.plan.isolated_root.as_path();
    let changed_paths = validated_workspace_changes(isolated, &workspace.plan)?;
    let diff_targets = diff_targets(isolated, declared_targets, &changed_paths);
    // Step 1: intent-to-add untracked declared targets so creates appear in diff.
    let mut intent_added: Vec<String> = Vec::new();
    for rel in &diff_targets {
        if isolated.join(&rel).exists() && !is_tracked(isolated, &rel) {
            run_git(&["add", "--intent-to-add", &rel], isolated).map_err(|e| {
                PatchError::GitDiffFailed {
                    stderr: e.to_string(),
                }
            })?;
            intent_added.push(rel.clone());
        }
    }
    // Step 2: ONE git diff for the apply patch (combined staged + unstaged;
    // never also --cached). --no-renames keeps a rename as delete+add so the
    // patch and the path accounting agree.
    let patch_bytes = run_diff(
        isolated,
        &["diff", "--binary", "--no-renames", "HEAD", "--"],
        &diff_targets,
    )?;
    // Step 3: derive paths from `--name-status -z` — NUL-separated, so paths
    // with spaces survive; status letters are authoritative for create/delete.
    let status_bytes = run_diff(
        isolated,
        &["diff", "--name-status", "--no-renames", "-z", "HEAD", "--"],
        &diff_targets,
    )?;
    let (changed_files, created_files, deleted_files) = parse_name_status(&status_bytes);
    let (pre_hashes, post_hashes) = target_hashes(isolated, declared_targets, baseline);
    // Step 6: best-effort cleanup of intent-to-add entries.
    for rel in &intent_added {
        let _ = run_git(&["reset", "HEAD", "--", rel], isolated);
    }
    Ok(CapturedPatch {
        patch_bytes,
        changed_files,
        created_files,
        deleted_files,
        pre_hashes,
        post_hashes,
        baseline_commit: workspace.baseline_commit.clone(),
    })
}

fn run_diff(isolated: &Path, prefix: &[&str], targets: &[String]) -> Result<Vec<u8>, PatchError> {
    let mut args: Vec<&str> = prefix.to_vec();
    args.extend(targets.iter().map(String::as_str));
    Ok(run_git(&args, isolated)
        .map_err(|e| PatchError::GitDiffFailed {
            stderr: e.to_string(),
        })?
        .stdout)
}

fn is_tracked(isolated: &Path, rel: &str) -> bool {
    run_git(&["ls-files", "--error-unmatch", rel], isolated).is_ok()
}

fn validated_workspace_changes(
    isolated: &Path,
    plan: &WritePlan,
) -> Result<Vec<String>, PatchError> {
    if !plan.workspace_boundary_required {
        return workspace_changed_paths(isolated);
    }
    let paths = workspace_changed_paths(isolated)?;
    for path in &paths {
        let normalized = normalize_target(&path, &plan.canonical_root)
            .map_err(|_| PatchError::UndeclaredWrite { path: path.clone() })?;
        if !path_is_owned(&normalized, plan) {
            return Err(PatchError::UndeclaredWrite { path: path.clone() });
        }
    }
    Ok(paths)
}

fn diff_targets(
    isolated: &Path,
    declared_targets: &[NormalizedPath],
    changed_paths: &[String],
) -> Vec<String> {
    if !changed_paths.is_empty() {
        return changed_paths.to_vec();
    }
    declared_targets
        .iter()
        .map(NormalizedPath::as_str)
        .filter(|path| !isolated.join(path).is_dir())
        .collect()
}

fn workspace_changed_paths(isolated: &Path) -> Result<Vec<String>, PatchError> {
    let mut out = Vec::new();
    let diff = run_git(
        &["diff", "--name-only", "--no-renames", "-z", "HEAD", "--"],
        isolated,
    )
    .map_err(|e| PatchError::GitDiffFailed {
        stderr: e.to_string(),
    })?;
    out.extend(split_nul_paths(&diff.stdout));
    let untracked = run_git(
        &["ls-files", "--others", "--exclude-standard", "-z"],
        isolated,
    )
    .map_err(|e| PatchError::GitDiffFailed {
        stderr: e.to_string(),
    })?;
    out.extend(split_nul_paths(&untracked.stdout));
    out.sort();
    out.dedup();
    Ok(out)
}

fn split_nul_paths(bytes: &[u8]) -> impl Iterator<Item = String> + '_ {
    bytes
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
        .map(|raw| String::from_utf8_lossy(raw).into_owned())
}

/// Parse `git diff --name-status -z` output: NUL-separated `STATUS\0path\0`
/// records (`--no-renames` guarantees no `R`/`C` two-path records). Robust for
/// paths containing spaces and for binary files (status, not diff text).
fn parse_name_status(bytes: &[u8]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let text = String::from_utf8_lossy(bytes);
    let mut fields = text.split('\0').filter(|s| !s.is_empty());
    let mut changed = Vec::new();
    let mut created = Vec::new();
    let mut deleted = Vec::new();
    while let Some(status) = fields.next() {
        let Some(path) = fields.next() else {
            break;
        };
        let path = path.to_string();
        match status.chars().next() {
            Some('A') => created.push(path.clone()),
            Some('D') => deleted.push(path.clone()),
            _ => {}
        }
        changed.push(path);
    }
    (changed, created, deleted)
}

/// Validate the captured patch against the declared contract (PRD §12).
pub fn validate_patch(
    captured: &CapturedPatch,
    plan: &WritePlan,
    cfg: &WriteCoordinatorConfig,
    agent_output_body: &str,
) -> Result<(), PatchError> {
    for file in &captured.changed_files {
        validate_changed_file(file, plan)?;
    }
    validate_size_budget(captured, plan, cfg)?;
    code_hygiene::validate(captured, plan, cfg)?;
    // VAL-WC-006 secret scan.
    if let Some((rule, line_preview)) = secret_scan::secret_scan(&captured.patch_bytes) {
        return Err(PatchError::SecretDetected { rule, line_preview });
    }
    if let Err(err) = crate::executor_output::ensure_output_usable(agent_output_body) {
        let detail = err.to_string();
        return Err(if captured.patch_bytes.is_empty() {
            PatchError::OutputNotUsable { detail }
        } else {
            PatchError::VerificationBlockedAfterPatch { detail }
        });
    }
    // VAL-WC-007 empty patch is an idempotent no-op when target hashes did not
    // change and the agent output is otherwise usable. Exact JSON remains
    // supported, but prose "no missing work" responses no longer brick a retry.
    if captured.patch_bytes.is_empty() {
        let idempotent = serde_json::from_str::<serde_json::Value>(agent_output_body)
            .ok()
            .and_then(|v| {
                v.get("idempotent_noop")
                    .and_then(serde_json::Value::as_bool)
            })
            .unwrap_or(false);
        let unchanged_targets = captured.pre_hashes == captured.post_hashes;
        if !(idempotent || unchanged_targets) {
            return Err(PatchError::EmptyPatch);
        }
    }
    // VAL-WC-004 deferred to patch_apply.rs (TASK-WC-006).
    Ok(())
}

/// VAL-WC-005 runtime byte budget (NOT the 500-line code-hygiene rule).
fn validate_size_budget(
    captured: &CapturedPatch,
    plan: &WritePlan,
    cfg: &WriteCoordinatorConfig,
) -> Result<(), PatchError> {
    let patch_len = captured.patch_bytes.len() as u64;
    if patch_len > cfg.max_patch_bytes {
        return Err(PatchError::PatchTooLarge {
            size: patch_len,
            max: cfg.max_patch_bytes,
        });
    }
    for file in &captured.changed_files {
        let size = std::fs::metadata(plan.isolated_root.join(file))
            .map(|m| m.len())
            .unwrap_or(0);
        if size > cfg.max_file_bytes {
            return Err(PatchError::FileTooLarge {
                path: file.clone(),
                size,
                max: cfg.max_file_bytes,
            });
        }
    }
    Ok(())
}

/// VAL-WC-001/002/003: declared, in-repo, no workspace-side symlink escape.
fn validate_changed_file(file: &str, plan: &WritePlan) -> Result<(), PatchError> {
    let normalized =
        normalize_target(file, &plan.canonical_root).map_err(|_| PatchError::UndeclaredWrite {
            path: file.to_string(),
        })?;
    if !path_is_owned(&normalized, plan) {
        return Err(PatchError::UndeclaredWrite {
            path: file.to_string(),
        });
    }
    let workspace_path = plan.isolated_root.join(file);
    let is_symlink = std::fs::symlink_metadata(&workspace_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_symlink {
        let link = std::fs::read_link(&workspace_path).map_err(|_| PatchError::SymlinkEscape {
            path: file.to_string(),
        })?;
        let resolved = if link.is_absolute() {
            link
        } else {
            workspace_path
                .parent()
                .unwrap_or(&plan.isolated_root)
                .join(link)
        };
        let canon = std::fs::canonicalize(&resolved).unwrap_or(resolved);
        if !canon.starts_with(&plan.canonical_root) {
            return Err(PatchError::SymlinkEscape {
                path: file.to_string(),
            });
        }
    }
    Ok(())
}

fn path_is_owned(path: &NormalizedPath, plan: &WritePlan) -> bool {
    plan.target_files
        .iter()
        .any(|target| normalized_path_overlaps(target, path))
        || plan
            .target_dir_scopes
            .iter()
            .any(|scope| normalized_path_overlaps(scope, path))
}

fn normalized_path_overlaps(left: &NormalizedPath, right: &NormalizedPath) -> bool {
    path_overlaps(&left.as_str(), &right.as_str())
}

fn path_overlaps(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// Write `<stage>/manifests/<item>.json` + `<stage>/patches/<item>.patch` atomically.
pub fn persist_manifest(
    run_root: &Path,
    run_id: &str,
    stage_id: &str,
    item_id: &ItemId,
    captured: &CapturedPatch,
    status: ManifestStatus,
) -> Result<PathBuf, PatchError> {
    let (manifest_path, patch_path) = manifest_paths(run_root, stage_id, item_id);
    create_parents(&manifest_path)?;
    create_parents(&patch_path)?;
    write_atomic(&patch_path, &captured.patch_bytes)?;

    let declared: Vec<String> = captured.post_hashes.keys().cloned().collect();
    let manifest = PatchManifest {
        schema: PATCH_MANIFEST_SCHEMA.to_string(),
        run_id: run_id.to_string(),
        stage_id: stage_id.to_string(),
        item_id: item_id.clone(),
        baseline_commit: captured.baseline_commit.clone(),
        patch_path: patch_path.clone(),
        declared_target_files: declared,
        changed_files: captured.changed_files.clone(),
        created_files: captured.created_files.clone(),
        deleted_files: captured.deleted_files.clone(),
        pre_hashes: captured.pre_hashes.clone(),
        post_hashes: captured.post_hashes.clone(),
        verify_command: None,
        agent_artifact_path: None,
        status,
    };
    write_manifest_json(&manifest_path, &manifest)?;
    Ok(manifest_path)
}

/// Rewrite ONLY the manifest JSON, leaving the patch file untouched.
pub fn persist_manifest_status_update(
    run_root: &Path,
    _run_id: &str,
    stage_id: &str,
    item_id: &ItemId,
    manifest: &PatchManifest,
) -> Result<(), PatchError> {
    let (manifest_path, _patch_path) = manifest_paths(run_root, stage_id, item_id);
    create_parents(&manifest_path)?;
    write_manifest_json(&manifest_path, manifest)
}

fn manifest_paths(run_root: &Path, stage_id: &str, item_id: &ItemId) -> (PathBuf, PathBuf) {
    let stage_root = run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id);
    let manifest = stage_root.join("manifests").join(format!("{item_id}.json"));
    let patch = stage_root.join("patches").join(format!("{item_id}.patch"));
    (manifest, patch)
}

fn create_parents(path: &Path) -> Result<(), PatchError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| PatchError::PersistFailed { source })?;
    }
    Ok(())
}

fn write_manifest_json(path: &Path, manifest: &PatchManifest) -> Result<(), PatchError> {
    let json = serde_json::to_vec_pretty(manifest)
        .map_err(|e| PatchError::PersistFailed { source: e.into() })?;
    write_atomic(path, &json)
}

/// Write to `<path>.tmp` then rename — atomic on POSIX.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PatchError> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&tmp, bytes).map_err(|source| PatchError::PersistFailed { source })?;
    std::fs::rename(&tmp, path).map_err(|source| PatchError::PersistFailed { source })?;
    Ok(())
}

#[cfg(test)]
#[path = "patch_manifest_d15_tests.rs"]
mod d15_tests;
#[cfg(test)]
#[path = "patch_manifest_line_count_tests.rs"]
mod line_count_tests;
#[cfg(test)]
#[path = "patch_manifest_tests.rs"]
mod tests;
