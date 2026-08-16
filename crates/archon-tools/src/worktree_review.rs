//! What an isolated agent actually produced (#184 M7).
//!
//! An isolated agent's work lands on a branch in a directory nobody looked at.
//! Without a summary the operator's only options are to trust it or to go
//! reading, and neither scales past two agents — so a completed agent reports
//! its branch, how far it diverged, and how much it changed.
//!
//! Measured against the **merge base**, not against the base branch's tip. The
//! base moves while an agent works, and diffing against a moved tip attributes
//! everyone else's commits to this agent.

use git2::{Oid, Repository};

use crate::git::diff::DiffStats;
use crate::worktree_manager::WorktreeInfo;

/// A summary of one isolated agent's work.
///
/// `DiffStats` carries no `PartialEq`, so the derive stops at `Clone`. Tests
/// assert on the rendered summary and the individual counts, which is the
/// contract that matters anyway.
#[derive(Debug, Clone)]
pub struct WorktreeReview {
    pub branch_name: String,
    pub base_branch: String,
    /// Commits on this branch that the base does not have.
    pub ahead: usize,
    /// Commits on the base that this branch does not have — how stale it is.
    pub behind: usize,
    pub stats: DiffStats,
}

impl WorktreeReview {
    /// Whether there is anything to merge.
    pub fn has_work(&self) -> bool {
        self.ahead > 0 || self.stats.files_changed > 0
    }

    /// One line for a completion envelope or a listing row.
    pub fn describe(&self) -> String {
        if !self.has_work() {
            return format!("branch '{}' — no changes", self.branch_name);
        }

        let mut out = format!(
            "branch '{}' — {} file{} changed, +{} -{}",
            self.branch_name,
            self.stats.files_changed,
            if self.stats.files_changed == 1 {
                ""
            } else {
                "s"
            },
            self.stats.insertions,
            self.stats.deletions,
        );
        if self.ahead > 0 {
            out.push_str(&format!(", {} ahead", self.ahead));
        }
        // Only worth saying when it is true: a branch behind its base is the
        // one whose merge is about to be interesting.
        if self.behind > 0 {
            out.push_str(&format!(", {} behind {}", self.behind, self.base_branch));
        }
        out
    }
}

/// Summarise `info`'s branch against the branch it was cut from.
///
/// `None` when either end cannot be resolved — a branch that was deleted, or a
/// repository that cannot be opened. A missing summary is reported as missing
/// rather than as an empty diff, because "nothing changed" and "we could not
/// tell" are different answers and only one of them means it is safe to
/// discard.
pub fn review_worktree(repo: &Repository, info: &WorktreeInfo) -> Option<WorktreeReview> {
    let head = branch_oid(repo, &info.branch_name)?;
    let base = branch_oid(repo, &info.original_branch)?;

    // Against the fork point, so commits the base gained meanwhile are not
    // attributed to this agent.
    let merge_base = repo.merge_base(head, base).ok()?;

    let (ahead, behind) = repo.graph_ahead_behind(head, base).unwrap_or((0, 0));

    let base_tree = repo.find_commit(merge_base).ok()?.tree().ok()?;
    let head_tree = repo.find_commit(head).ok()?.tree().ok()?;
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None)
        .ok()?;
    let stats = diff.stats().ok()?;

    Some(WorktreeReview {
        branch_name: info.branch_name.clone(),
        base_branch: info.original_branch.clone(),
        ahead,
        behind,
        stats: DiffStats {
            files_changed: stats.files_changed(),
            insertions: stats.insertions(),
            deletions: stats.deletions(),
        },
    })
}

/// Summarise `info` by opening its worktree, for callers without git2.
///
/// Exists so `archon-core` can report an isolated agent's work without taking a
/// git2 dependency for one call.
pub fn review_for(info: &WorktreeInfo) -> Option<WorktreeReview> {
    let repo = Repository::open(&info.worktree_path)
        .or_else(|_| Repository::open(&info.original_dir))
        .ok()?;
    review_worktree(&repo, info)
}

fn branch_oid(repo: &Repository, name: &str) -> Option<Oid> {
    repo.find_branch(name, git2::BranchType::Local)
        .ok()?
        .get()
        .target()
}

#[cfg(test)]
#[path = "worktree_review_tests.rs"]
mod tests;
