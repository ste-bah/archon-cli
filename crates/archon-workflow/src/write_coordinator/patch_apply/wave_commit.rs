//! Land an accepted wave's declared outputs as a commit.
//!
//! Every item workspace is cut from HEAD, so an output left uncommitted is
//! invisible to every later wave: a task that creates a file its dependent
//! consumes has, from the next worktree's perspective, produced nothing. That
//! makes dependency ordering — the reason waves exist — silently do nothing,
//! and pushes the dependent agent into recreating its own inputs, which
//! ownership validation then correctly rejects.
//!
//! Only the manifests' own recorded write-set is staged, by explicit path. A
//! target repository normally carries unrelated work in progress, and a wave
//! commit that swept that up would be worse than the bug it fixes.

use std::collections::BTreeSet;
use std::path::Path;

use crate::write_coordinator::WaveId;
use crate::write_coordinator::patch_manifest::PatchManifest;
use crate::write_coordinator::worktree_isolation::run_git;

use super::ApplyError;

/// The union of every manifest's recorded write-set — the same paths ownership
/// validation already accepted for these items, and nothing else.
fn wave_paths(manifests: &[PatchManifest]) -> Vec<String> {
    let mut paths: BTreeSet<String> = BTreeSet::new();
    for manifest in manifests {
        paths.extend(manifest.changed_files.iter().cloned());
        paths.extend(manifest.created_files.iter().cloned());
        paths.extend(manifest.deleted_files.iter().cloned());
    }
    paths.into_iter().collect()
}

fn git_or_commit_error(args: &[&str], canonical_root: &Path) -> Result<Vec<u8>, ApplyError> {
    run_git(args, canonical_root)
        .map(|output| output.stdout)
        .map_err(|err| ApplyError::WaveCommitFailed {
            stderr: err.to_string(),
        })
}

/// Commit the wave's declared outputs, advancing the baseline later waves are
/// cut from. Returns `Ok(())` when the wave changed nothing on disk.
pub(super) fn commit_wave_outputs(
    canonical_root: &Path,
    manifests: &[PatchManifest],
    run_id: &str,
    stage_id: &str,
    wave_id: WaveId,
) -> Result<(), ApplyError> {
    let paths = wave_paths(manifests);
    if paths.is_empty() {
        return Ok(());
    }

    // Stage by explicit path. `git add` also stages removals for named paths,
    // which covers a manifest's deleted files.
    let mut add_args: Vec<&str> = vec!["add", "--"];
    add_args.extend(paths.iter().map(String::as_str));
    git_or_commit_error(&add_args, canonical_root)?;

    // A wave whose patches were byte-identical to what is already committed
    // has nothing to record; committing anyway would fail.
    let mut status_args: Vec<&str> = vec!["status", "--porcelain", "--"];
    status_args.extend(paths.iter().map(String::as_str));
    if git_or_commit_error(&status_args, canonical_root)?
        .iter()
        .all(u8::is_ascii_whitespace)
    {
        return Ok(());
    }

    let message = format!("archon: wave {wave_id} outputs (run {run_id}, stage {stage_id})");
    // The pathspec keeps this a partial commit: anything else staged in the
    // target repository's index is left exactly as the operator had it.
    let mut commit_args: Vec<&str> = vec![
        "-c",
        "user.name=archon-workflow",
        "-c",
        "user.email=archon-workflow@local",
        "commit",
        "--no-gpg-sign",
        "--no-verify",
        "-m",
        &message,
        "--",
    ];
    commit_args.extend(paths.iter().map(String::as_str));
    git_or_commit_error(&commit_args, canonical_root)?;
    Ok(())
}
