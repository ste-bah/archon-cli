//! Formatter noise outside a write item's scope: found, restored, never fatal.
//!
//! A tree-wide formatter run by a write agent (`cargo fmt --all`, `black .`)
//! rewrites files the item never meant to own. Each of those is an undeclared
//! change, and an undeclared change fails the branch at capture
//! (`UndeclaredWrite`), which skips every dependent wave. Issue-13, live on
//! wf-7db01ce7 `agents-3-0`: sixty-four files outside `target_files`, the
//! branch's real work stranded in its worktree.
//!
//! The rule here is narrow and language-agnostic: an UNDECLARED path whose
//! worktree bytes equal the canonical bytes once ASCII whitespace is removed
//! from both is restored to its baseline content, so the patch never carries
//! it, and the restored paths are reported as a review gap. A declared path is
//! never touched — reformatting a file the item owns is legitimate work — and
//! a path that differs by anything but whitespace is a real change, judged by
//! the ownership gates exactly as before.
//!
//! The comparison is against the canonical file because the worktree was
//! created from the canonical tree, dirty state included, and nothing in the
//! wave has applied yet: the canonical file IS what the agent started from.
//! The restore is `git checkout HEAD -- <path>` in the worktree, whose `HEAD`
//! is the sealed baseline commit of that same content.

use std::path::Path;

use archon_write_plan::{WritePlan, normalize_target};

use super::patch_manifest::{path_is_owned, workspace_changed_paths};
use super::worktree_isolation::run_git;

/// Whether the change from `before` to `after` is whitespace-only: the two
/// files differ, but not once ASCII whitespace is removed from both. Either
/// side unreadable — a created or deleted file — is a real change. Byte-
/// identical files are not whitespace-only either: there is nothing to drop.
pub fn whitespace_only_change(before: &Path, after: &Path) -> bool {
    let (Ok(before), Ok(after)) = (std::fs::read(before), std::fs::read(after)) else {
        return false;
    };
    before != after && without_whitespace(&before) == without_whitespace(&after)
}

fn without_whitespace(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect()
}

/// What the worktree holds versus the sealed baseline, partitioned by what
/// the plan says about each path. Repo-relative, sorted, disjoint.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorktreeChanges {
    /// Changed paths the plan owns: a declared target or under a declared
    /// directory scope.
    pub declared: Vec<String>,
    /// Changed paths the plan does not own whose diff against the canonical
    /// file is real — not whitespace-only. Created and deleted files are here.
    pub undeclared: Vec<String>,
    /// Changed paths the plan does not own whose diff is whitespace-only.
    pub whitespace_only: Vec<String>,
}

/// Every path changed in the worktree versus the sealed baseline commit —
/// tracked modified, added or deleted, plus untracked files `.gitignore` does
/// not cover (`git diff HEAD` and `git ls-files --others --exclude-standard`,
/// the same scan capture uses) — partitioned by the plan.
///
/// This is the ONE scan of the worktree the scope grant reads: the candidate
/// set for a grant is what the agent actually changed, not what it reported
/// (Issue-16), and the whitespace-only subset is what Issue-13 drops.
///
/// Best effort by design: a worktree that is not a git checkout (unit-test
/// fixtures), a git failure, or a path the coordinator cannot name yields
/// nothing for it, and every such path then meets the ownership gates as it
/// does today. The failure mode is the status quo.
pub fn worktree_changes(plan: &WritePlan) -> WorktreeChanges {
    let Ok(changed) = workspace_changed_paths(&plan.isolated_root) else {
        return WorktreeChanges::default();
    };
    let mut out = WorktreeChanges::default();
    for path in changed {
        // Ownership is judged on the normalised path; the path itself is kept
        // as git names it, which is how capture and the restore address it.
        let Ok(normalized) = normalize_target(&path, &plan.canonical_root) else {
            continue;
        };
        if path_is_owned(&normalized, plan) {
            out.declared.push(path);
        } else if whitespace_only_change(
            &plan.canonical_root.join(&path),
            &plan.isolated_root.join(&path),
        ) {
            out.whitespace_only.push(path);
        } else {
            out.undeclared.push(path);
        }
    }
    for set in [
        &mut out.declared,
        &mut out.undeclared,
        &mut out.whitespace_only,
    ] {
        set.sort();
        set.dedup();
    }
    out
}

/// Every path changed in the worktree that the plan does not own and whose
/// diff against the canonical file is whitespace-only. Repo-relative, sorted.
/// The whitespace-only partition of [`worktree_changes`].
pub fn undeclared_whitespace_only_changes(plan: &WritePlan) -> Vec<String> {
    worktree_changes(plan).whitespace_only
}

/// Restore `paths` in the worktree to the sealed baseline (`HEAD`), index and
/// working copy alike, so a later `git diff HEAD` no longer lists them.
///
/// Returns the paths actually restored. One that cannot be — not in the
/// baseline commit, or git refusing it — is left as it is and omitted, so it
/// meets the ownership gates as it does today rather than vanishing silently.
pub fn restore_to_baseline(isolated_root: &Path, paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| run_git(&["checkout", "HEAD", "--", path], isolated_root).is_ok())
        .cloned()
        .collect()
}

/// [`restore_to_baseline`] for paths that may not exist in the baseline at
/// all: the out-of-scope drop (Issue-27), whose candidates include files the
/// agent CREATED. `git checkout HEAD -- <path>` refuses a pathspec HEAD does
/// not hold, so a created file is unstaged if it was staged and removed from
/// disk instead; a modified or deleted one is restored exactly as before.
///
/// Returns the paths actually dropped. One that is neither in the baseline
/// nor on disk has nothing to drop and is omitted.
pub fn restore_or_remove(isolated_root: &Path, paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| {
            if run_git(&["checkout", "HEAD", "--", path], isolated_root).is_ok() {
                return true;
            }
            let on_disk = isolated_root.join(path);
            if !on_disk.is_file() {
                return false;
            }
            // Not in HEAD: a created file. Staged or not, `ls-files --others`
            // must stop listing it, and the scan capture runs must not see it.
            let _ = run_git(
                &["rm", "-q", "--cached", "--force", "--", path],
                isolated_root,
            );
            std::fs::remove_file(&on_disk).is_ok()
        })
        .cloned()
        .collect()
}
