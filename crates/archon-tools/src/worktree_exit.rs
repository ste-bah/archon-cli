//! Leaving a worktree: keep, discard, or merge back.
//!
//! Split out of `worktree_manager.rs` to keep both files under the 500-line
//! FileSizeGuard threshold. Creation and ownership live there; teardown and
//! merge live here.

use std::fs;

use git2::{BranchType, Repository};

use crate::worktree_manager::{ExitAction, WorktreeInfo, WorktreeManager};
use crate::worktree_ownership;

impl WorktreeManager {
    /// Exit a worktree with the specified action.
    pub fn exit_worktree(
        repo: &Repository,
        info: &WorktreeInfo,
        action: ExitAction,
    ) -> Result<String, String> {
        match action {
            ExitAction::Keep => Ok(format!(
                "Worktree kept at {} on branch '{}'",
                info.worktree_path.display(),
                info.branch_name,
            )),
            ExitAction::Discard => {
                prune_worktree(repo, &info.branch_name)?;
                remove_worktree_dir(info)?;
                delete_branch(repo, &info.branch_name)?;

                Ok(format!(
                    "Worktree discarded: branch '{}' and directory removed",
                    info.branch_name,
                ))
            }
            ExitAction::Merge => {
                merge_worktree_branch(repo, info)?;

                prune_worktree(repo, &info.branch_name)?;
                remove_worktree_dir(info)?;
                delete_branch(repo, &info.branch_name)?;

                Ok(format!(
                    "Worktree merged: branch '{}' integrated into '{}'",
                    info.branch_name, info.original_branch,
                ))
            }
        }
    }
}

impl WorktreeManager {
    /// Act on the worktree owned by `owner_id`, opening its repository first.
    ///
    /// Exists so callers without git2 — the `/worktrees` command among them —
    /// can merge or discard without taking the dependency for one call, and so
    /// the liveness refusal lives in one place rather than at every caller.
    pub fn exit_by_owner(owner_id: &str, action: ExitAction) -> Result<String, String> {
        let root = Self::worktrees_dir();
        if worktree_ownership::owner_liveness(&root, owner_id)
            == worktree_ownership::OwnerLiveness::Foreign
        {
            return Err(format!(
                "worktree '{owner_id}' is still in use by {}",
                worktree_ownership::describe_owner(&root, owner_id)
            ));
        }

        let info = Self::find_by_owner(owner_id)
            .ok_or_else(|| format!("no worktree owned by '{owner_id}'"))?;

        // The base repository, not the worktree: a discard removes the worktree
        // out from under an open handle, and git2 needs somewhere to prune from
        // afterwards.
        let repo = Repository::open(&info.original_dir)
            .map_err(|e| format!("cannot open the base repository: {e}"))?;

        Self::exit_worktree(&repo, &info, action)
    }
}

/// Remove the directory and give up ownership of it.
///
/// The lock and marker are dropped only once the tree is actually gone, so a
/// failure here leaves the ownership record intact rather than orphaning a
/// directory nothing claims.
fn remove_worktree_dir(info: &WorktreeInfo) -> Result<(), String> {
    if info.worktree_path.exists() {
        fs::remove_dir_all(&info.worktree_path)
            .map_err(|e| format!("Failed to remove worktree directory: {e}"))?;
    }
    worktree_ownership::forget(&WorktreeManager::worktrees_dir(), &info.owner_id);
    Ok(())
}

fn delete_branch(repo: &Repository, branch_name: &str) -> Result<(), String> {
    if let Ok(mut branch) = repo.find_branch(branch_name, BranchType::Local) {
        branch
            .delete()
            .map_err(|e| format!("Failed to delete branch: {e}"))?;
    }
    Ok(())
}

/// Prune a worktree reference from git.
///
/// `pub(crate)` because the reclaim path in `worktree_manager` needs it too:
/// removing an abandoned directory without pruning git's registration leaves
/// the replacement unable to create its branch.
pub(crate) fn prune_worktree(repo: &Repository, branch_name: &str) -> Result<(), String> {
    let wt_name = crate::worktree_manager::branch_name_to_worktree_name(branch_name);
    if let Ok(wt) = repo.find_worktree(&wt_name) {
        let valid = wt.validate().is_ok();
        wt.prune(Some(
            git2::WorktreePruneOptions::new()
                .valid(valid)
                .locked(false)
                .working_tree(true),
        ))
        .map_err(|e| format!("Failed to prune worktree: {e}"))?;
    }
    Ok(())
}

/// Merge the worktree branch into the original branch.
fn merge_worktree_branch(repo: &Repository, info: &WorktreeInfo) -> Result<(), String> {
    let wt_branch = repo
        .find_branch(&info.branch_name, BranchType::Local)
        .map_err(|e| format!("Failed to find worktree branch: {e}"))?;
    let wt_commit_oid = wt_branch
        .get()
        .target()
        .ok_or_else(|| "Worktree branch has no target".to_string())?;
    let wt_commit = repo
        .find_commit(wt_commit_oid)
        .map_err(|e| format!("Failed to find worktree commit: {e}"))?;

    let head = repo
        .head()
        .map_err(|e| format!("Failed to get HEAD: {e}"))?;
    let head_commit = head
        .peel_to_commit()
        .map_err(|e| format!("Failed to peel HEAD to commit: {e}"))?;

    // Fast-forward where the worktree commit already descends from HEAD.
    if repo
        .graph_descendant_of(wt_commit_oid, head_commit.id())
        .unwrap_or(false)
    {
        let refname = format!("refs/heads/{}", info.original_branch);
        repo.reference(
            &refname,
            wt_commit_oid,
            true,
            &format!("archon: fast-forward merge of {}", info.branch_name),
        )
        .map_err(|e| format!("Failed to fast-forward: {e}"))?;

        let obj = repo
            .find_object(wt_commit_oid, None)
            .map_err(|e| format!("Failed to find object: {e}"))?;
        repo.checkout_tree(&obj, None)
            .map_err(|e| format!("Failed to checkout after fast-forward: {e}"))?;
        repo.set_head(&refname)
            .map_err(|e| format!("Failed to set HEAD: {e}"))?;

        return Ok(());
    }

    // Otherwise a real merge commit.
    let annotated = repo
        .find_annotated_commit(wt_commit_oid)
        .map_err(|e| format!("Failed to create annotated commit: {e}"))?;

    let mut merge_opts = git2::MergeOptions::new();
    let mut checkout_opts = git2::build::CheckoutBuilder::new();
    checkout_opts.force();

    repo.merge(
        &[&annotated],
        Some(&mut merge_opts),
        Some(&mut checkout_opts),
    )
    .map_err(|e| format!("Merge failed: {e}"))?;

    let mut index = repo
        .index()
        .map_err(|e| format!("Failed to get index: {e}"))?;

    if index.has_conflicts() {
        return Err("Merge has conflicts — manual resolution required".to_string());
    }

    let tree_oid = index
        .write_tree()
        .map_err(|e| format!("Failed to write tree: {e}"))?;
    let tree = repo
        .find_tree(tree_oid)
        .map_err(|e| format!("Failed to find tree: {e}"))?;

    let sig = repo
        .signature()
        .or_else(|_| git2::Signature::now("Archon", "archon@localhost"))
        .map_err(|e| format!("Failed to create signature: {e}"))?;

    let msg = format!(
        "archon: merge {} into {}",
        info.branch_name, info.original_branch
    );

    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        &msg,
        &tree,
        &[&head_commit, &wt_commit],
    )
    .map_err(|e| format!("Failed to create merge commit: {e}"))?;

    repo.cleanup_state()
        .map_err(|e| format!("Failed to cleanup state: {e}"))?;

    Ok(())
}
