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

/// Every path changed in the worktree that the plan does not own and whose
/// diff against the canonical file is whitespace-only. Repo-relative, sorted.
///
/// Best effort by design: a worktree that is not a git checkout (unit-test
/// fixtures) or a git failure yields nothing, and every path then meets the
/// ownership gates as it does today. The failure mode is the status quo.
pub fn undeclared_whitespace_only_changes(plan: &WritePlan) -> Vec<String> {
    let Ok(changed) = workspace_changed_paths(&plan.isolated_root) else {
        return Vec::new();
    };
    let mut out: Vec<String> = changed
        .into_iter()
        .filter(|path| {
            normalize_target(path, &plan.canonical_root)
                .is_ok_and(|normalized| !path_is_owned(&normalized, plan))
        })
        .filter(|path| {
            whitespace_only_change(
                &plan.canonical_root.join(path),
                &plan.isolated_root.join(path),
            )
        })
        .collect();
    out.sort();
    out.dedup();
    out
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
