//! TASK-WC-005 — Patch capture from the isolated workspace + validation manifest.
//!
//! After the agent finishes, capture its patch with a SINGLE `git diff --binary
//! HEAD -- <targets>` (already combines staged + unstaged), validate it against
//! the declared contract before it touches canonical, and persist the durable
//! manifest + patch evidence.

mod secret_scan;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::ItemId;
use super::worktree_isolation::{CanonicalBaseline, ItemWorkspace, run_git};
use super::write_plan::{NormalizedPath, WritePlan, normalize_target};
use crate::write_coordinator::WriteCoordinatorConfig;

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
    #[error("patch path '{path}' escapes repository via symlink")]
    SymlinkEscape { path: String },
    #[error("file '{path}' is {size} bytes, exceeds max {max}")]
    FileTooLarge { path: String, size: u64, max: u64 },
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
    // Step 1: intent-to-add untracked declared targets so creates appear in diff.
    let mut intent_added: Vec<String> = Vec::new();
    for target in declared_targets {
        let rel = target.as_str();
        if isolated.join(&rel).exists() && !is_tracked(isolated, &rel) {
            run_git(&["add", "--intent-to-add", &rel], isolated).map_err(|e| {
                PatchError::GitDiffFailed {
                    stderr: e.to_string(),
                }
            })?;
            intent_added.push(rel);
        }
    }
    reject_undeclared_workspace_changes(isolated, &workspace.plan, declared_targets)?;
    // Step 2: ONE git diff for the apply patch (combined staged + unstaged;
    // never also --cached). --no-renames keeps a rename as delete+add so the
    // patch and the path accounting agree.
    let targets: Vec<String> = declared_targets
        .iter()
        .map(NormalizedPath::as_str)
        .collect();
    let patch_bytes = run_diff(
        isolated,
        &["diff", "--binary", "--no-renames", "HEAD", "--"],
        &targets,
    )?;
    // Step 3: derive paths from `--name-status -z` — NUL-separated, so paths
    // with spaces survive; status letters are authoritative for create/delete.
    let status_bytes = run_diff(
        isolated,
        &["diff", "--name-status", "--no-renames", "-z", "HEAD", "--"],
        &targets,
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

fn reject_undeclared_workspace_changes(
    isolated: &Path,
    plan: &WritePlan,
    declared_targets: &[NormalizedPath],
) -> Result<(), PatchError> {
    if !plan.workspace_boundary_required {
        return Ok(());
    }
    let declared: BTreeSet<NormalizedPath> = declared_targets.iter().cloned().collect();
    for path in workspace_changed_paths(isolated)? {
        let normalized = normalize_target(&path, &plan.canonical_root)
            .map_err(|_| PatchError::UndeclaredWrite { path: path.clone() })?;
        if !declared.contains(&normalized) {
            return Err(PatchError::UndeclaredWrite { path });
        }
    }
    Ok(())
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

type HashPair = (BTreeMap<String, String>, BTreeMap<String, String>);

/// pre_hashes from baseline; post_hashes from the post-agent workspace state.
fn target_hashes(
    isolated: &Path,
    declared_targets: &[NormalizedPath],
    baseline: &CanonicalBaseline,
) -> HashPair {
    let mut pre = BTreeMap::new();
    let mut post = BTreeMap::new();
    for target in declared_targets {
        let rel = target.as_str();
        if let Some(meta) = baseline.declared_target_meta.get(&rel) {
            pre.insert(rel.clone(), meta.blake3_hex.clone());
        }
        post.insert(rel.clone(), hash_file_or_deleted(&isolated.join(&rel)));
    }
    (pre, post)
}

fn hash_file_or_deleted(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => blake3::hash(&bytes).to_hex().to_string(),
        Err(_) => "deleted".to_string(),
    }
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
    // VAL-WC-006 secret scan.
    if let Some((rule, line_preview)) = secret_scan::secret_scan(&captured.patch_bytes) {
        return Err(PatchError::SecretDetected { rule, line_preview });
    }
    // VAL-WC-007 empty patch only with declared idempotent_noop + matching hashes.
    if captured.patch_bytes.is_empty() {
        let idempotent = serde_json::from_str::<serde_json::Value>(agent_output_body)
            .ok()
            .and_then(|v| {
                v.get("idempotent_noop")
                    .and_then(serde_json::Value::as_bool)
            })
            .unwrap_or(false);
        if !(idempotent && captured.pre_hashes == captured.post_hashes) {
            return Err(PatchError::EmptyPatch);
        }
    }
    // VAL-WC-008 reuse the canonical output-usability gate.
    crate::executor_output::ensure_output_usable(agent_output_body).map_err(|e| {
        PatchError::OutputNotUsable {
            detail: e.to_string(),
        }
    })?;
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
    if !plan.target_files.contains(&normalized) {
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
#[path = "patch_manifest_tests.rs"]
mod tests;
