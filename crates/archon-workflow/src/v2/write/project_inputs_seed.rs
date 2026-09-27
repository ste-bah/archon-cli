//! Seed a write branch's worktree with the project's acceptance inputs,
//! and capture what the branch changed there (Batch E; the records and the
//! placement rule are `write_coordinator::project_inputs`, the landing is
//! `patch_apply::project_inputs_apply`).
//!
//! Seeded: every regular file under an input that the worktree's git
//! ignores and does not track, copied from the project root with its state
//! there recorded as the file's baseline. A path git carries is left to git
//! -- a tracked file is the repository's and lands as a patch -- and a path
//! git would see but does not carry is never seeded, so nothing seeded can
//! enter the patch. A link is never followed or copied. A host dependency
//! share on the way (`ignored_deps` mirrors an ignored directory by linking
//! its children into the canonical checkout) is replaced by a private
//! directory, so a seeded file is never written through into the checkout.
//! Nothing is seeded past the byte cap; what is not seeded is recorded and
//! told to the agent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::write_coordinator::project_inputs::{
    CaptureRecord, InputChange, ProjectInputPolicy, SeedRecord, capture_path, captured_bytes_dir,
    file_state, read_json, read_no_follow, seed_path, write_file, write_json,
};
use crate::write_coordinator::worktree_isolation::{check_ignore, run_git};
use crate::{WorkflowError, WorkflowResult};

/// Most files one input tree contributes.
const MAX_FILES: usize = 50_000;

fn io_error(path: &Path, error: std::io::Error) -> WorkflowError {
    WorkflowError::io(path, error)
}

fn git_error(error: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::StageFailed(format!("project input seeding: {error}"))
}

/// Every regular file under `root/rel`, relative, sorted; links and other
/// non-files under it are recorded in `skipped` (never followed).
fn walk(
    root: &Path,
    rel: &Path,
    policy: &ProjectInputPolicy,
    out: &mut Vec<String>,
    skipped: &mut Vec<(String, String)>,
) {
    if policy.excluded(rel) || out.len() >= MAX_FILES {
        return;
    }
    let path = root.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return;
    };
    let name = rel.to_string_lossy().into_owned();
    if meta.is_dir() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            skipped.push((name, "unreadable directory".into()));
            return;
        };
        let mut children: Vec<_> = entries.flatten().map(|e| e.file_name()).collect();
        children.sort();
        for child in children {
            walk(root, &rel.join(child), policy, out, skipped);
        }
    } else if meta.is_file() {
        out.push(name);
    } else {
        skipped.push((name, "not a regular file (a link is never followed)".into()));
    }
}

/// The inputs' files the worktree's git ignores and does not track.
fn git_free(worktree: &Path, files: &[String]) -> WorkflowResult<BTreeSet<String>> {
    Ok(check_ignore(worktree, files)
        .map_err(git_error)?
        .into_iter()
        .collect())
}

/// Make every directory on the way to `rel` in the worktree a private real
/// directory: an ignored, untracked link there is a host share and is
/// removed; any other link or a file in the way refuses the path.
fn private_parents(worktree: &Path, rel: &str) -> Result<(), String> {
    let mut cursor = PathBuf::new();
    let parts: Vec<&str> = rel.split('/').collect();
    for (at, part) in parts.iter().enumerate() {
        cursor.push(part);
        let path = worktree.join(&cursor);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let spelled = cursor.to_string_lossy().into_owned();
                let share = check_ignore(worktree, std::slice::from_ref(&spelled))
                    .map(|found| !found.is_empty())
                    .unwrap_or(false);
                if !share {
                    return Err(format!("{spelled} is a link git carries"));
                }
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
            Ok(meta) if at + 1 < parts.len() && !meta.is_dir() => {
                return Err(format!("{} is not a directory", cursor.display()));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Seed `worktree` for (`stage_id`, `item_id`) and record it; `None` when
/// the run records no project inputs.
pub(super) fn seed(
    run_root: &Path,
    stage_id: &str,
    item_id: &str,
    worktree: &Path,
) -> WorkflowResult<Option<SeedRecord>> {
    let seed_file = seed_path(run_root, stage_id, item_id);
    let Some(policy) = ProjectInputPolicy::for_run(run_root) else {
        let _ = std::fs::remove_file(&seed_file);
        return Ok(None);
    };
    let mut record = SeedRecord {
        project: policy.project.clone(),
        worktree: worktree.to_path_buf(),
        ..SeedRecord::default()
    };
    let mut budget = policy.limit;
    for input in &policy.inputs {
        let rel = input.to_string_lossy().into_owned();
        if policy.excluded(input) {
            record
                .skipped
                .push((rel, "excluded from project inputs".into()));
            continue;
        }
        record.inputs.push(rel.clone());
        // Whole-ignored only when git says so plainly; anything it cannot
        // answer (a link on the way it will not look beyond) is not.
        if let Err(why) = private_parents(worktree, &rel) {
            record.skipped.push((rel.clone(), why));
        }
        let untracked = run_git(&["ls-files", "-z", "--", &rel], worktree)
            .is_ok_and(|output| output.stdout.is_empty());
        let whole = [rel.clone(), format!("{rel}/")];
        if untracked && git_free(worktree, &whole).is_ok_and(|free| !free.is_empty()) {
            record.ignored_roots.push(rel.clone());
        }
        let mut files = Vec::new();
        walk(
            &policy.project,
            input,
            &policy,
            &mut files,
            &mut record.skipped,
        );
        if files.len() >= MAX_FILES {
            record.skipped.push((
                rel.clone(),
                format!("more than {MAX_FILES} files: the rest were not copied"),
            ));
        }
        // Private directories first: git answers nothing about a path beyond
        // a link, and a host share on the way is replaced, never followed.
        let mut candidates = Vec::new();
        for file in files {
            let parent = Path::new(&file)
                .parent()
                .map(|p| p.to_string_lossy().into_owned());
            match parent
                .filter(|p| !p.is_empty())
                .map(|p| private_parents(worktree, &p))
            {
                Some(Err(why)) => record.skipped.push((file, why)),
                _ => candidates.push(file),
            }
        }
        let free = git_free(worktree, &candidates)?;
        for file in candidates {
            if !free.contains(&file) {
                record.skipped.push((
                    file,
                    "the repository tracks it or would see it: git carries it, not the seed".into(),
                ));
                continue;
            }
            let source = policy.project.join(&file);
            let size = std::fs::symlink_metadata(&source).map_or(u64::MAX, |m| m.len());
            if size > budget {
                record
                    .skipped
                    .push((file, "over the project input size cap".into()));
                continue;
            }
            let Ok(bytes) = read_no_follow(&source) else {
                record
                    .skipped
                    .push((file, "unreadable in the project root".into()));
                continue;
            };
            if let Err(why) = private_parents(worktree, &file) {
                record.skipped.push((file, why));
                continue;
            }
            let destination = worktree.join(&file);
            write_file(worktree, &destination, &bytes).map_err(|e| io_error(&destination, e))?;
            budget = budget.saturating_sub(bytes.len() as u64);
            let state = blake3::hash(&bytes).to_hex().to_string();
            record.files.insert(file, state);
        }
    }
    write_json(&seed_file, &record).map_err(|e| io_error(&seed_file, e))?;
    Ok(Some(record))
}

/// The worktree paths the branch may write as project data: each input its
/// git ignores whole, and each seeded file.
pub(super) fn writable(record: &SeedRecord) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = record
        .ignored_roots
        .iter()
        .map(|root| record.worktree.join(root))
        .collect();
    for file in record.files.keys() {
        if !record
            .ignored_roots
            .iter()
            .any(|root| Path::new(file).starts_with(root))
        {
            paths.push(record.worktree.join(file));
        }
    }
    paths
}

/// What the agent is told about its copy of the project's data.
pub(super) fn preamble(record: &SeedRecord) -> String {
    if record.inputs.is_empty() {
        return String::new();
    }
    let mut text = format!(
        "\nProject data: the acceptance checks run against the project's own data under {} \
         (relative to the project root). This worktree holds a copy of it at the same relative \
         paths ({} file(s)). Run the product's own commands against it as the checks do. What \
         you change there is applied to the project root when this branch lands; it is never \
         part of your git patch, and a file the project's copy changed after this worktree was \
         seeded is refused and reported, not overwritten.",
        record.inputs.join(", "),
        record.files.len()
    );
    if !record.skipped.is_empty() {
        let listed: Vec<String> = record
            .skipped
            .iter()
            .take(10)
            .map(|(path, why)| format!("{path} ({why})"))
            .collect();
        text.push_str(&format!(
            " Not copied: {}{}.",
            listed.join("; "),
            if record.skipped.len() > 10 {
                format!("; and {} more", record.skipped.len() - 10)
            } else {
                String::new()
            }
        ));
    }
    text.push('\n');
    text
}

/// Capture what the branch changed under its seeded inputs, keep the bytes
/// for the landing and record them; returns the changed paths. Nothing is
/// recorded when the branch was not seeded or changed nothing.
pub(super) fn capture(
    run_root: &Path,
    stage_id: &str,
    item_id: &str,
    worktree: &Path,
    task_ids: &[String],
) -> WorkflowResult<Vec<String>> {
    let record_file = capture_path(run_root, stage_id, item_id);
    let bytes_dir = captured_bytes_dir(run_root, stage_id, item_id);
    let _ = std::fs::remove_file(&record_file);
    let _ = std::fs::remove_dir_all(&bytes_dir);
    let (Some(seed), Some(policy)) = (
        read_json::<SeedRecord>(&seed_path(run_root, stage_id, item_id)),
        ProjectInputPolicy::for_run(run_root),
    ) else {
        return Ok(Vec::new());
    };
    let mut present = Vec::new();
    let mut ignored_skips = Vec::new();
    for input in &seed.inputs {
        walk(
            worktree,
            Path::new(input),
            &policy,
            &mut present,
            &mut ignored_skips,
        );
    }
    if present.len() >= MAX_FILES {
        return Err(WorkflowError::StageFailed(format!(
            "project inputs in the worktree hold more than {MAX_FILES} files; the changes cannot all be captured"
        )));
    }
    let free = git_free(worktree, &present)?;
    let mut changes: BTreeMap<String, InputChange> = BTreeMap::new();
    let mut total = 0u64;
    for rel in present.iter().filter(|rel| free.contains(*rel)) {
        let post = file_state(&worktree.join(rel));
        let baseline = seed
            .files
            .get(rel)
            .cloned()
            .unwrap_or_else(|| "absent".into());
        if post == baseline {
            continue;
        }
        let bytes =
            read_no_follow(&worktree.join(rel)).map_err(|e| io_error(&worktree.join(rel), e))?;
        total += bytes.len() as u64;
        if total > policy.limit {
            return Err(WorkflowError::StageFailed(format!(
                "project input changes exceed the {} byte cap at {rel}",
                policy.limit
            )));
        }
        let kept = bytes_dir.join(rel);
        if let Some(parent) = kept.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        std::fs::write(&kept, &bytes).map_err(|e| io_error(&kept, e))?;
        changes.insert(rel.clone(), InputChange { baseline, post });
    }
    // Only a seeded file that is GONE was deleted: one the agent replaced by
    // a link or anything else is no file the landing may copy, and the
    // project root's copy is left alone.
    let present: BTreeSet<&String> = present.iter().collect();
    for (rel, baseline) in &seed.files {
        let gone = matches!(
            std::fs::symlink_metadata(worktree.join(rel)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        );
        if !present.contains(rel) && gone {
            changes.insert(
                rel.clone(),
                InputChange {
                    baseline: baseline.clone(),
                    post: "deleted".into(),
                },
            );
        }
    }
    if changes.is_empty() {
        return Ok(Vec::new());
    }
    let paths: Vec<String> = changes.keys().cloned().collect();
    let record = CaptureRecord {
        task_ids: task_ids.to_vec(),
        changes,
    };
    write_json(&record_file, &record).map_err(|e| io_error(&record_file, e))?;
    Ok(paths)
}

/// Forget a capture whose branch produced no manifest: nothing of it lands.
pub(super) fn discard(run_root: &Path, stage_id: &str, item_id: &str) {
    let _ = std::fs::remove_file(capture_path(run_root, stage_id, item_id));
    let _ = std::fs::remove_dir_all(captured_bytes_dir(run_root, stage_id, item_id));
}

/// Gap id prefix of a branch whose project-input changes were refused.
pub(crate) const PROJECT_INPUT_REFUSED_GAP_PREFIX: &str = "project_inputs_refused_";

/// A landing whose project-input changes were refused has not delivered what
/// its branch reported: the item is downgraded, as an unapplied patch is,
/// with a HIGH gap for its tasks naming what was refused and why.
pub(super) fn report_refusals(
    artifacts: &mut super::WorktreeWaveArtifacts,
    refusals: &[(crate::write_coordinator::ItemId, String)],
) {
    for (item_id, reason) in refusals {
        let Some(index) = artifacts
            .completed
            .iter()
            .position(|branch| branch.item_id.as_str() == item_id.as_str())
        else {
            continue;
        };
        let Some(result) = artifacts.results.get_mut(index) else {
            continue;
        };
        result.status = crate::v2::WorkflowV2Status::NeedsReview;
        if let Some(data) = result.data.as_object_mut() {
            data.insert("project_inputs_refused".into(), serde_json::json!(reason));
        }
        result.residual_gaps.push(crate::v2::WorkflowV2ResidualGap {
            id: format!("{PROJECT_INPUT_REFUSED_GAP_PREFIX}{item_id}"),
            description: format!(
                "this branch's changes to the project's acceptance inputs were NOT applied to the project root: {reason}. What its patch carried landed; its project data did not. Re-run the data commands in a fresh worktree, which is seeded with the project's current data."
            ),
            severity: Some("high".to_string()),
        });
    }
}

#[cfg(test)]
#[path = "project_inputs_seed_tests.rs"]
mod tests;
