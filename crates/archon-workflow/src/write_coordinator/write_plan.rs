//! TASK-WC-002 — WritePlan model, path normalization, resource keys (PRD-012 §8, §10.1).
//!
//! Normalization happens ONCE here; downstream coordinator modules only see
//! clean repo-relative forward-slash paths and deterministic resource keys.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::ItemId;

/// WC-ERR-* taxonomy (PRD-012 §19).
#[derive(Debug, Error)]
pub enum WritePlanError {
    #[error("invalid target path: {0}")]
    InvalidTargetPath(String),
    #[error("item declares no target files from any source")]
    MissingTargets,
    #[error("target path escapes repository via '..': {0}")]
    TraversalEscape(String),
    #[error("absolute target path outside repository: {0}")]
    AbsoluteEscape(String),
    #[error("target path resolves through a symlink leaving the repository: {0}")]
    SymlinkEscape(String),
    #[error("target path contains an empty segment or trailing slash: {0}")]
    EmptySegment(String),
    #[error("malformed target glob: {0}")]
    MalformedGlob(String),
    #[error("baseline id must start with 'blake3:' or 'git:': {0}")]
    InvalidBaselineId(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Repo-relative path with forward slashes, original casing preserved.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NormalizedPath(PathBuf);

impl NormalizedPath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Forward-slash string form (lossless for paths produced by normalize_target).
    pub fn as_str(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }

    /// Resource-key string: case-folded on case-insensitive filesystems.
    pub fn key_string(&self) -> String {
        fold_case_for_os(&self.as_str(), std::env::consts::OS)
    }
}

/// Conflict-graph resource key. Variant declaration order IS the Ord order:
/// File < Dir < Glob (asserted by test).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ResourceKey {
    File(String),
    Dir(String),
    Glob(String),
}

/// Provenance of an item's resolved target files (PRD-012 §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetFilesSource {
    Item,
    ItemExpected,
    StageLevel,
}

/// One coordinated implementation item's write contract (PRD-012 §8.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WritePlan {
    pub run_id: String,
    pub stage_id: String,
    pub item_id: ItemId,
    pub canonical_root: PathBuf,
    pub isolated_root: PathBuf,
    pub target_files: Vec<NormalizedPath>,
    pub target_files_source: TargetFilesSource,
    pub read_context_files: Vec<NormalizedPath>,
    pub verify_inputs: Vec<NormalizedPath>,
    pub baseline_id: String,
    pub workspace_boundary_required: bool,
    pub resource_keys: BTreeSet<ResourceKey>,
}

/// Validate + normalize one raw target path against the canonical root.
pub fn normalize_target(
    raw: &str,
    canonical_root: &Path,
) -> Result<NormalizedPath, WritePlanError> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(WritePlanError::InvalidTargetPath(raw.replace('\0', "\\0")));
    }
    // Unify separators first: empty-segment and `..` checks must see every
    // form, and std::path::Component has no Empty variant to catch `//`.
    let unified = raw.replace('\\', "/");
    if unified.split('/').any(|seg| seg == "..") {
        return Err(WritePlanError::TraversalEscape(unified));
    }
    if unified.contains("//") || unified.ends_with('/') {
        return Err(WritePlanError::EmptySegment(unified));
    }
    let path = PathBuf::from(&unified);
    let rel = if path.is_absolute() {
        match path.strip_prefix(canonical_root) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => return Err(WritePlanError::AbsoluteEscape(unified)),
        }
    } else {
        path
    };
    let real = realpath_within(canonical_root, &rel)?;
    let display = real.to_string_lossy().replace('\\', "/");
    Ok(NormalizedPath(PathBuf::from(display)))
}

/// Guard against symlink loops (`a -> b -> a`) and pathological nesting.
const MAX_SYMLINK_DEPTH: u32 = 40;

/// Resolve `rel` against `canonical_root`, fully following every intermediate
/// symlink (including chains and not-yet-existing targets), and reject any
/// result that leaves the repository.
fn realpath_within(canonical_root: &Path, rel: &Path) -> Result<PathBuf, WritePlanError> {
    // Only Normal components survive normalize_target's earlier checks.
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(WritePlanError::InvalidTargetPath(
            rel.to_string_lossy().into_owned(),
        ));
    }
    let root =
        std::fs::canonicalize(canonical_root).unwrap_or_else(|_| canonical_root.to_path_buf());
    let resolved = resolve_abs(&root.join(rel), 0)?;
    let final_rel = resolved
        .strip_prefix(&root)
        .map_err(|_| WritePlanError::SymlinkEscape(resolved.to_string_lossy().into_owned()))?;
    Ok(final_rel.to_path_buf())
}

/// Walk an absolute path component-by-component, dereferencing symlinks
/// (recursively, against the live filesystem) and resolving `..`/`.` by
/// popping. Non-existent leaf components are kept verbatim so future files
/// validate. Containment is NOT enforced here — the caller does that on the
/// fully-resolved result so a chained symlink cannot smuggle an escape past it.
fn resolve_abs(path: &Path, depth: u32) -> Result<PathBuf, WritePlanError> {
    if depth > MAX_SYMLINK_DEPTH {
        return Err(WritePlanError::SymlinkEscape(
            path.to_string_lossy().into_owned(),
        ));
    }
    let mut acc = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                acc.pop();
            }
            Component::Prefix(_) | Component::RootDir => acc.push(comp.as_os_str()),
            Component::Normal(seg) => {
                let next = acc.join(seg);
                let is_symlink = std::fs::symlink_metadata(&next)
                    .map(|meta| meta.file_type().is_symlink())
                    .unwrap_or(false);
                if is_symlink {
                    let target = std::fs::read_link(&next)?;
                    let candidate = if target.is_absolute() {
                        target
                    } else {
                        acc.join(target)
                    };
                    acc = resolve_abs(&candidate, depth + 1)?;
                } else {
                    acc = next;
                }
            }
        }
    }
    Ok(acc)
}

/// §8.1 priority: item.target_files → item.expected_target_files → stage level.
pub fn resolve_target_files(
    item_payload: &serde_json::Value,
    stage_expected: &[String],
) -> Result<(Vec<String>, TargetFilesSource), WritePlanError> {
    for (key, source) in [
        ("target_files", TargetFilesSource::Item),
        ("expected_target_files", TargetFilesSource::ItemExpected),
    ] {
        let Some(arr) = item_payload.get(key).and_then(serde_json::Value::as_array) else {
            continue;
        };
        if arr.is_empty() {
            continue;
        }
        let files: Vec<String> = arr
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(String::from)
            .collect();
        if files.is_empty() {
            return Err(WritePlanError::InvalidTargetPath(format!(
                "item `{key}` contains no string entries"
            )));
        }
        return Ok((files, source));
    }
    if !stage_expected.is_empty() {
        return Ok((stage_expected.to_vec(), TargetFilesSource::StageLevel));
    }
    Err(WritePlanError::MissingTargets)
}

/// Build the deterministic resource-key set for one item.
pub fn resource_keys_for_targets(
    targets: &[NormalizedPath],
    canonical_root: &Path,
    declared_globs: &[String],
) -> Result<BTreeSet<ResourceKey>, WritePlanError> {
    let os = std::env::consts::OS;
    let mut keys = BTreeSet::new();
    for target in targets {
        keys.insert(ResourceKey::File(target.key_string()));
        // Dir keys only for parents the item CREATES (absent from canonical).
        let Some(parent) = target.as_path().parent() else {
            continue;
        };
        let mut ancestor = PathBuf::new();
        for comp in parent.components() {
            ancestor.push(comp);
            if !canonical_root.join(&ancestor).exists() {
                let dir = ancestor.to_string_lossy().replace('\\', "/");
                keys.insert(ResourceKey::Dir(fold_case_for_os(&dir, os)));
            }
        }
    }
    for glob in declared_globs {
        globset::Glob::new(glob)
            .map_err(|err| WritePlanError::MalformedGlob(format!("{glob}: {err}")))?;
        keys.insert(ResourceKey::Glob(fold_case_for_os(glob, os)));
    }
    Ok(keys)
}

/// Deterministic overlap table (PRD-012 §10.1).
pub fn keys_conflict(a: &ResourceKey, b: &ResourceKey) -> bool {
    use ResourceKey::{Dir, File, Glob};
    match (a, b) {
        (File(x), File(y)) => x == y,
        (File(f), Dir(d)) | (Dir(d), File(f)) => f == d || f.starts_with(&format!("{d}/")),
        (Dir(x), Dir(y)) => {
            x == y || x.starts_with(&format!("{y}/")) || y.starts_with(&format!("{x}/"))
        }
        (Glob(g), File(f)) | (File(f), Glob(g)) => glob_match(g, f),
        (Glob(g), Dir(d)) | (Dir(d), Glob(g)) => glob_match(g, &format!("{d}/*")),
        (Glob(x), Glob(y)) => globs_overlap(x, y),
    }
}

fn glob_match(pattern: &str, candidate: &str) -> bool {
    match globset::Glob::new(pattern) {
        Ok(glob) => glob.compile_matcher().is_match(candidate),
        // Malformed patterns are conservatively conflicting; construction-time
        // validation in resource_keys_for_targets reports them as errors.
        Err(_) => true,
    }
}

/// Textual prefix overlap (PRD-012 §10.1): conservative, deterministic.
fn globs_overlap(a: &str, b: &str) -> bool {
    let pa = literal_prefix(a);
    let pb = literal_prefix(b);
    pa.starts_with(pb) || pb.starts_with(pa)
}

fn literal_prefix(glob: &str) -> &str {
    match glob.find(['*', '?', '[', '{']) {
        Some(idx) => &glob[..idx],
        None => glob,
    }
}

/// Baseline ids are namespaced by hash kind: `blake3:<hash>` or `git:<sha>`.
pub fn parse_baseline_id(raw: &str) -> Result<String, WritePlanError> {
    if raw.starts_with("blake3:") || raw.starts_with("git:") {
        Ok(raw.to_string())
    } else {
        Err(WritePlanError::InvalidBaselineId(raw.to_string()))
    }
}

/// Case-fold for case-insensitive filesystems; identity elsewhere.
fn fold_case_for_os(s: &str, os: &str) -> String {
    if os == "macos" || os == "windows" {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
#[path = "write_plan_tests.rs"]
mod tests;
