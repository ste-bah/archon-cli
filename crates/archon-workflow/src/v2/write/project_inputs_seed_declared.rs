//! Batch G2: seed and capture a write branch's copies of the project
//! artifacts its call declared outside the project's inputs (the placement
//! and the landing rule are `write_coordinator::project_inputs_declared`).
//!
//! Each is one file: copied from the project root into the branch's copy
//! (or recorded `absent`), with the project root's state as its baseline;
//! what the branch leaves there is captured beside its manifest and landed
//! by `patch_apply::project_inputs_apply`, which refuses a stale baseline.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{git_free, io_error, private_parents};
use crate::WorkflowResult;
use crate::write_coordinator::project_inputs::{
    DeclaredCopy, InputChange, ProjectInputPolicy, SeedRecord, file_state, meta_state,
    read_no_follow, refuse_links, write_file,
};

/// Why a declared artifact has no seeded copy when the project IS the
/// repository and git carries the path: the worktree file is its copy, and
/// the branch's patch lands it.
pub(in crate::v2::write) const CARRIED_BY_PATCH: &str =
    "the repository carries it: it lands through this branch's patch";

/// What a branch's call declared as project artifacts, relative to the
/// project root, and the checkout its worktree was made from.
pub(in crate::v2::write) struct DeclaredArtifacts<'a> {
    pub(in crate::v2::write) canonical_root: &'a Path,
    pub(in crate::v2::write) rels: &'a [String],
}

fn same_tree(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Where the branch's copy of `rel` goes, or why it has none here: the
/// worktree when its git ignores and does not track the path; nowhere when
/// the project is the repository and git carries it (the patch lands it);
/// otherwise the branch's staging directory.
fn placement(
    worktree: &Path,
    staging: &Path,
    project: &Path,
    canonical_root: &Path,
    rel: &str,
) -> WorkflowResult<Result<PathBuf, String>> {
    let parent = Path::new(rel)
        .parent()
        .map(|p| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
        .filter(|p| !p.is_empty());
    let private = parent.map_or(Ok(()), |parent| private_parents(worktree, &parent));
    let free = private.is_ok()
        && private_parents(worktree, rel).is_ok()
        && git_free(worktree, &[rel.to_string()])?.contains(rel);
    Ok(if free {
        Ok(worktree.join(rel))
    } else if same_tree(project, canonical_root) {
        Err(CARRIED_BY_PATCH.into())
    } else {
        Ok(staging.join(rel))
    })
}

/// Seed the branch's copy of each declared project artifact outside the
/// inputs into `record.declared`; anything that cannot be is named in
/// `record.skipped`, which the agent is told.
pub(super) fn seed_declared(
    policy: &ProjectInputPolicy,
    record: &mut SeedRecord,
    declared: &DeclaredArtifacts<'_>,
    staging: &Path,
    budget: &mut u64,
) -> WorkflowResult<()> {
    let worktree = record.worktree.clone();
    for rel in declared.rels {
        if policy.covers(rel) {
            continue;
        }
        if let Err(why) = policy.placed(rel, true) {
            let why = format!("a declared project artifact no landing writes: {why}");
            record.skipped.push((rel.clone(), why));
            continue;
        }
        let source = policy.project.join(rel);
        if let Err(error) = refuse_links(&policy.project, &source) {
            record
                .skipped
                .push((rel.clone(), format!("not read: {error}")));
            continue;
        }
        let copy = match placement(
            &worktree,
            staging,
            &policy.project,
            declared.canonical_root,
            rel,
        )? {
            Ok(copy) => copy,
            Err(why) => {
                record.skipped.push((rel.clone(), why));
                continue;
            }
        };
        let root = if copy.starts_with(&worktree) {
            worktree.as_path()
        } else {
            staging
        };
        // What the worktree held there (an ignored copy of the repository's
        // own) is not the project's: the branch starts from the project's.
        match std::fs::symlink_metadata(&copy) {
            Ok(meta) if meta.is_file() || meta.file_type().is_symlink() => {
                std::fs::remove_file(&copy).map_err(|e| io_error(&copy, e))?;
            }
            Ok(_) => {
                let why = "a directory stands at the branch's copy".to_string();
                record.skipped.push((rel.clone(), why));
                continue;
            }
            Err(_) => {}
        }
        if let Some(parent) = copy.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        let baseline = match std::fs::symlink_metadata(&source) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent".to_string(),
            Ok(meta) if meta.is_file() && meta.len() <= *budget => match read_no_follow(&source) {
                Ok(bytes) => {
                    write_file(root, &copy, &bytes).map_err(|e| io_error(&copy, e))?;
                    *budget = budget.saturating_sub(bytes.len() as u64);
                    blake3::hash(&bytes).to_hex().to_string()
                }
                Err(_) => {
                    let why = "unreadable in the project root: regenerate it at the branch's copy";
                    record.skipped.push((rel.clone(), why.into()));
                    meta_state(&source)
                }
            },
            Ok(meta) if meta.is_file() => {
                let why = "over the project input size cap: regenerate it at the branch's copy";
                record.skipped.push((rel.clone(), why.into()));
                meta_state(&source)
            }
            _ => {
                let why = "not a regular file in the project root".to_string();
                record.skipped.push((rel.clone(), why));
                continue;
            }
        };
        record
            .declared
            .insert(rel.clone(), DeclaredCopy { copy, baseline });
    }
    Ok(())
}

/// The branch's declared copies it may write: each copy in the worktree by
/// name, and the directory of each staged one (created atomically through a
/// sibling, inside the branch's own staging directory).
pub(super) fn writable(record: &SeedRecord) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for copy in record.declared.values() {
        let path = if copy.copy.starts_with(&record.worktree) {
            Some(copy.copy.clone())
        } else {
            copy.copy.parent().map(Path::to_path_buf)
        };
        if let Some(path) = path.filter(|path| !paths.contains(path)) {
            paths.push(path);
        }
    }
    paths
}

/// Capture what the branch left at each declared copy, judged against its
/// seed baseline, never through a link: kept under `bytes.0` for the landing
/// and recorded in `changes` and `declared`. `Some(reason)` when the total
/// went past the cap (nothing of the branch's data then lands).
pub(super) fn capture_declared(
    policy: &ProjectInputPolicy,
    seed: &SeedRecord,
    forbidden: &archon_write_plan::ForbiddenPaths,
    bytes: (&Path, &mut u64),
    changes: &mut BTreeMap<String, InputChange>,
    declared: &mut BTreeSet<String>,
    dropped: &mut Vec<(String, String)>,
) -> WorkflowResult<Option<String>> {
    let (bytes_dir, total) = bytes;
    for (rel, copy) in &seed.declared {
        let post = file_state(&copy.copy);
        let copied = !copy.baseline.starts_with("meta:") && copy.baseline != "absent";
        let post = match post.as_str() {
            _ if post == copy.baseline => continue,
            // Never copied into the branch and not regenerated: unchanged.
            "absent" if !copied => continue,
            "absent" => "deleted".to_string(),
            marker if marker.starts_with('<') => {
                dropped.push((rel.clone(), "not a regular file".into()));
                continue;
            }
            _ => post,
        };
        if forbidden.matches(rel) {
            dropped.push((rel.clone(), "a path the branch's tasks forbid".into()));
            continue;
        }
        if let Err(why) = policy.placed(rel, true) {
            dropped.push((rel.clone(), format!("no landing writes it: {why}")));
            continue;
        }
        if post != "deleted" {
            let content = read_no_follow(&copy.copy).map_err(|e| io_error(&copy.copy, e))?;
            *total += content.len() as u64;
            if *total > policy.limit {
                return Ok(Some(format!(
                    "the project data changes exceed the {} byte cap (at {rel})",
                    policy.limit
                )));
            }
            let kept = bytes_dir.join(rel);
            if let Some(parent) = kept.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
            }
            std::fs::write(&kept, &content).map_err(|e| io_error(&kept, e))?;
        }
        let baseline = copy.baseline.clone();
        changes.insert(rel.clone(), InputChange { baseline, post });
        declared.insert(rel.clone());
    }
    Ok(None)
}
