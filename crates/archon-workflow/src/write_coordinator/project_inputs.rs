//! The project's acceptance inputs inside a write branch (Batch E).
//!
//! Acceptance runs every frozen check in a scratch copy of the target
//! repository with the acceptance policy's `project_inputs` -- project data
//! directories under the project root -- copied over it
//! (`acceptance_scratch`). A write branch's worktree held none of that data,
//! and since Issue-124 its shell cannot write the project root either, so a
//! check that needs project data produced by the product's own data
//! commands could never be made to pass by any branch.
//!
//! So each write branch's worktree is seeded with a copy of exactly those
//! inputs (`v2::write::project_inputs_seed`), at the same relative paths.
//! Only paths the worktree's git ignores and does not track are seeded, so
//! none of it can enter the git patch; a tracked path stays git's and lands
//! as a patch. What the branch changes there is captured beside its manifest
//! and applied to the project root when the branch lands
//! (`patch_apply::project_inputs_apply`), under the Issue-113 rules extended
//! to directories: a per-file baseline taken at seed time (a file the
//! project root changed since is refused as stale), no engine-loaded
//! namespace, no link followed, a size cap, one landing at a time under the
//! repository lock, an append-only log, and undo on failure.
//!
//! This module holds what both sides read: the policy, the records and
//! where they live, and the rule for where a copy may be placed.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Most bytes one branch is seeded with, and most one landing applies.
pub const MAX_PROJECT_INPUT_BYTES: u64 = 1 << 30;

/// The acceptance policy's project inputs, as the run recorded them at
/// launch (`v2/generated-metadata.json`, the observer snapshot's native
/// execution binding): the same set acceptance scratch copies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInputPolicy {
    /// The project root, canonical.
    pub project: PathBuf,
    pub inputs: Vec<PathBuf>,
    pub excludes: Vec<PathBuf>,
    /// The task set root, never written by a landing.
    pub task_root: PathBuf,
    /// The byte cap: [`MAX_PROJECT_INPUT_BYTES`], or the policy's own
    /// scratch cap when smaller.
    pub limit: u64,
}

impl ProjectInputPolicy {
    /// The run's policy, or `None` when it recorded none, it does not
    /// validate, or it names a project the run directory is not inside (a
    /// copied run directory must never reach the project it was copied
    /// from).
    pub fn for_run(run_root: &Path) -> Option<Self> {
        let bytes = std::fs::read(run_root.join("v2/generated-metadata.json")).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let policy = value.pointer("/observer_snapshot/native_execution/policy")?;
        let policy: crate::acceptance_scratch::ScratchPolicy =
            serde_json::from_value(policy.clone()).ok()?;
        policy.validate().ok()?;
        if policy.project_inputs.is_empty() {
            return None;
        }
        let project = policy.project.canonicalize().ok()?;
        if !run_root.canonicalize().ok()?.starts_with(&project) {
            return None;
        }
        Some(Self {
            project,
            inputs: policy.project_inputs,
            excludes: policy.project_input_excludes,
            // Compared with canonical destinations: canonical when it exists.
            task_root: policy.task_root.canonicalize().unwrap_or(policy.task_root),
            limit: policy.scratch_bytes.min(MAX_PROJECT_INPUT_BYTES),
        })
    }

    /// Whether `rel` is never copied out of, or into, the project.
    pub fn excluded(&self, rel: &Path) -> bool {
        crate::acceptance_scratch::project_input_excluded(rel, &self.excludes)
    }

    /// Whether `rel` lies under one of the inputs.
    pub fn covers(&self, rel: &str) -> bool {
        let rel = Path::new(rel);
        self.inputs.iter().any(|input| rel.starts_with(input)) && !self.excluded(rel)
    }

    /// Where a landing may place `rel` under the project root, or why not:
    /// a clean relative path under an input, never excluded, never inside
    /// the task set, never `.git`, and never in a `.archon/` namespace the
    /// engine loads from (`materialize_scope::ENGINE_LOADED`).
    pub fn destination(&self, rel: &str) -> Result<PathBuf, String> {
        let path = Path::new(rel);
        let clean = !rel.is_empty()
            && path.components().all(|c| matches!(c, Component::Normal(_)))
            && !path.components().any(|c| c.as_os_str() == ".git");
        if !clean {
            return Err("not a clean relative path".into());
        }
        if !self.covers(rel) {
            return Err("outside the acceptance policy's project inputs".into());
        }
        let parts: Vec<String> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
            .collect();
        if parts.first().map(String::as_str) == Some(".archon") {
            let namespace_ok = parts.len() >= 3
                && parts[1].is_ascii()
                && !parts[1].starts_with('.')
                && !crate::write_coordinator::patch_apply::ENGINE_LOADED
                    .contains(&parts[1].as_str());
            if !namespace_ok {
                return Err(format!(
                    "`.archon/{}` is a namespace the engine loads from",
                    parts.get(1).cloned().unwrap_or_default()
                ));
            }
        }
        let destination = self.project.join(path);
        if destination.starts_with(&self.task_root) {
            return Err("inside the task set, which no landing writes".into());
        }
        Ok(destination)
    }
}

/// A file's state as the records name it: its blake3 hash, `absent`, or a
/// marker for anything that is not a regular file (a link included).
pub fn file_state(path: &Path) -> String {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent".into(),
        Ok(meta) if meta.is_file() => read_no_follow(path)
            .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
            .unwrap_or_else(|_| "<unreadable>".into()),
        Ok(_) => "<not a file>".into(),
        Err(_) => "<unreadable>".into(),
    }
}

/// Read a regular file without following a final link.
pub fn read_no_follow(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes)?;
    Ok(bytes)
}

/// What the host seeded one branch's worktree with.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedRecord {
    pub project: PathBuf,
    pub worktree: PathBuf,
    /// The inputs, relative: where the capture looks for changes.
    pub inputs: Vec<String>,
    /// The inputs the worktree's git ignores whole: every path under them
    /// is the project's data, writable in the worktree.
    pub ignored_roots: Vec<String>,
    /// Each seeded file and the project root's state of it when seeded:
    /// the baseline its landing must still find there.
    pub files: BTreeMap<String, String>,
    /// What was not seeded, and why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<(String, String)>,
}

/// One project input a branch changed: the project root's state of it when
/// the branch was seeded (`absent` for a file the branch created) and the
/// branch's own (`deleted` for one it removed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputChange {
    pub baseline: String,
    pub post: String,
}

/// What a landing must apply for one branch: its changes, and the tasks the
/// branch served (whom a refusal is routed to).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub task_ids: Vec<String>,
    pub changes: BTreeMap<String, InputChange>,
}

fn item_dir(run_root: &Path, stage_id: &str, item_id: &str) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id)
        .join("project-inputs")
        .join(item_id)
}

pub fn seed_path(run_root: &Path, stage_id: &str, item_id: &str) -> PathBuf {
    item_dir(run_root, stage_id, item_id).join("seed.json")
}

pub fn capture_path(run_root: &Path, stage_id: &str, item_id: &str) -> PathBuf {
    item_dir(run_root, stage_id, item_id).join("capture.json")
}

/// Where a captured file's bytes are kept until its landing.
pub fn captured_bytes_dir(run_root: &Path, stage_id: &str, item_id: &str) -> PathBuf {
    item_dir(run_root, stage_id, item_id).join("files")
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

/// Every existing component of `path` below `root` is a real directory or
/// file: a link is refused, never followed.
pub fn refuse_links(root: &Path, path: &Path) -> std::io::Result<()> {
    let inside = path
        .strip_prefix(root)
        .map_err(|_| std::io::Error::other("outside its root"))?;
    let mut cursor = root.to_path_buf();
    for component in inside.components() {
        cursor.push(component);
        match std::fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(std::io::Error::other(format!(
                    "{} is a symlink",
                    cursor.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Write `bytes` at `destination` (under `root`) by rename, following no
/// link; returns what was there.
pub fn write_file(
    root: &Path,
    destination: &Path,
    bytes: &[u8],
) -> std::io::Result<Option<Vec<u8>>> {
    refuse_links(root, destination)?;
    let before = match std::fs::symlink_metadata(destination) {
        Ok(meta) if meta.is_file() => Some(read_no_follow(destination)?),
        Ok(_) => return Err(std::io::Error::other("exists and is not a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let parent = destination
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    refuse_links(root, parent)?;
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = parent.join(format!(".{name}.{}.archon-input.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    std::io::Write::write_all(&mut file, bytes)?;
    drop(file);
    if let Err(error) = std::fs::rename(&temporary, destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(before)
}

/// A run's recorded acceptance policy naming `inputs` of `project`, as the
/// launch snapshot records it: for tests that need a seeded run.
#[cfg(test)]
pub(crate) fn write_test_policy(run_root: &Path, project: &Path, inputs: &[&str]) {
    let policy = serde_json::json!({
        "repository": project, "project": project, "task_root": project.join("tasks"),
        "scratch_parent": "/nonexistent/archon-scratch", "project_inputs": inputs,
        "combined": true, "toolchain_path": "/usr/bin:/bin", "environment": {},
        "cargo_seed": null, "timeout_secs": 10, "output_bytes": 1024, "scratch_bytes": 1u64 << 30,
    });
    let metadata =
        serde_json::json!({"observer_snapshot": {"native_execution": {"policy": policy}}});
    write_json(&run_root.join("v2/generated-metadata.json"), &metadata).unwrap();
}

#[cfg(test)]
#[path = "project_inputs_tests.rs"]
mod tests;
