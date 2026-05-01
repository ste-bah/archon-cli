use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use git2::{BranchType, Repository};

/// Information about a single worktree session.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub session_id: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub original_dir: PathBuf,
    pub original_branch: String,
    pub created_at: DateTime<Utc>,
}

/// Action to take when exiting a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitAction {
    /// Merge the worktree branch into the original branch.
    Merge,
    /// Keep the worktree and branch as-is.
    Keep,
    /// Delete the worktree directory and branch.
    Discard,
}

/// Manages git worktree lifecycle for agent sessions.
pub struct WorktreeManager {
    #[allow(dead_code)]
    worktrees: HashMap<String, WorktreeInfo>,
}

impl WorktreeManager {
    pub fn new() -> Self {
        Self {
            worktrees: HashMap::new(),
        }
    }

    /// Return the base directory for all archon worktrees.
    ///
    /// Defaults to `~/.local/share/archon/worktrees/`.
    pub fn worktrees_dir() -> PathBuf {
        dirs_next().join("archon").join("worktrees")
    }

    /// Create a new worktree for the given session.
    ///
    /// The worktree is placed at `<worktrees_dir>/<session_id>/` and a branch
    /// named `archon/<session_id_short>` is created pointing at the current HEAD.
    pub fn create_worktree(repo: &Repository, session_id: &str) -> Result<WorktreeInfo, String> {
        let wt_dir = Self::worktrees_dir().join(session_id);

        // git2 requires the target directory to NOT exist; it creates it itself.
        // Ensure the parent directory exists.
        if let Some(parent) = wt_dir.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create worktrees parent directory: {e}"))?;
        }
        // Remove stale directory if it exists (from previous failed runs)
        if wt_dir.exists() {
            fs::remove_dir_all(&wt_dir)
                .map_err(|e| format!("Failed to remove stale worktree directory: {e}"))?;
        }

        let original_branch = crate::git::current_branch(repo)?;
        let original_dir = repo
            .workdir()
            .ok_or_else(|| "Repository has no workdir (bare repo)".to_string())?
            .to_path_buf();

        let short = &session_id[..8.min(session_id.len())];
        let branch_name = format!("archon/{short}");

        // Create branch at HEAD
        let head = repo
            .head()
            .map_err(|e| format!("Failed to get HEAD: {e}"))?;
        let commit = head
            .peel_to_commit()
            .map_err(|e| format!("Failed to peel HEAD to commit: {e}"))?;
        repo.branch(&branch_name, &commit, false)
            .map_err(|e| format!("Failed to create branch '{branch_name}': {e}"))?;

        // Add the worktree using git2
        repo.worktree(
            &branch_name_to_worktree_name(&branch_name),
            &wt_dir,
            Some(
                git2::WorktreeAddOptions::new().reference(Some(
                    &repo
                        .find_branch(&branch_name, BranchType::Local)
                        .map_err(|e| format!("Failed to find branch: {e}"))?
                        .into_reference(),
                )),
            ),
        )
        .map_err(|e| format!("Failed to create worktree: {e}"))?;

        // Write metadata file for list/cleanup
        let meta_path = wt_dir.join(".archon-worktree.json");
        let now = Utc::now();
        let meta = serde_json::json!({
            "session_id": session_id,
            "branch_name": branch_name,
            "original_dir": original_dir.to_string_lossy(),
            "original_branch": original_branch,
            "created_at": now.to_rfc3339(),
        });
        fs::write(&meta_path, meta.to_string())
            .map_err(|e| format!("Failed to write worktree metadata: {e}"))?;

        Ok(WorktreeInfo {
            session_id: session_id.to_string(),
            branch_name,
            worktree_path: wt_dir,
            original_dir,
            original_branch,
            created_at: now,
        })
    }

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
                // Prune the worktree from git's perspective
                prune_worktree(repo, &info.branch_name)?;

                // Remove the directory
                if info.worktree_path.exists() {
                    fs::remove_dir_all(&info.worktree_path)
                        .map_err(|e| format!("Failed to remove worktree directory: {e}"))?;
                }

                // Delete the branch
                if let Ok(mut branch) = repo.find_branch(&info.branch_name, BranchType::Local) {
                    branch
                        .delete()
                        .map_err(|e| format!("Failed to delete branch: {e}"))?;
                }

                Ok(format!(
                    "Worktree discarded: branch '{}' and directory removed",
                    info.branch_name,
                ))
            }
            ExitAction::Merge => {
                merge_worktree_branch(repo, info)?;

                // After merge, clean up worktree and branch
                prune_worktree(repo, &info.branch_name)?;

                if info.worktree_path.exists() {
                    fs::remove_dir_all(&info.worktree_path)
                        .map_err(|e| format!("Failed to remove worktree directory: {e}"))?;
                }

                if let Ok(mut branch) = repo.find_branch(&info.branch_name, BranchType::Local) {
                    branch
                        .delete()
                        .map_err(|e| format!("Failed to delete branch: {e}"))?;
                }

                Ok(format!(
                    "Worktree merged: branch '{}' integrated into '{}'",
                    info.branch_name, info.original_branch,
                ))
            }
        }
    }

    /// List all worktrees found in the worktrees directory.
    pub fn list_worktrees() -> Vec<WorktreeInfo> {
        let wt_dir = Self::worktrees_dir();
        let mut result = Vec::new();

        let entries = match fs::read_dir(&wt_dir) {
            Ok(e) => e,
            Err(_) => return result,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let meta_path = path.join(".archon-worktree.json");
            if let Ok(contents) = fs::read_to_string(&meta_path)
                && let Ok(meta) = serde_json::from_str::<serde_json::Value>(&contents) {
                    let info = WorktreeInfo {
                        session_id: meta["session_id"].as_str().unwrap_or_default().to_string(),
                        branch_name: meta["branch_name"].as_str().unwrap_or_default().to_string(),
                        worktree_path: path.clone(),
                        original_dir: PathBuf::from(
                            meta["original_dir"].as_str().unwrap_or_default(),
                        ),
                        original_branch: meta["original_branch"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        created_at: meta["created_at"]
                            .as_str()
                            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                            .unwrap_or_else(Utc::now),
                    };
                    result.push(info);
                }
        }

        result
    }

    /// Convenience wrapper: open repo at `working_dir` and create a worktree.
    pub fn create_worktree_from_path(
        working_dir: &std::path::Path,
        session_id: &str,
    ) -> Result<WorktreeInfo, String> {
        let repo = Repository::open(working_dir).map_err(|e| {
            format!(
                "Failed to open repository at {}: {e}",
                working_dir.display()
            )
        })?;
        Self::create_worktree(&repo, session_id)
    }

    /// Cleanup a worktree session if it has no uncommitted changes.
    pub fn cleanup_session(session_id: &str) -> Result<(), String> {
        let wt_path = Self::worktrees_dir().join(session_id);
        if !wt_path.exists() {
            return Ok(());
        }

        // Check for uncommitted changes
        if let Ok(wt_repo) = Repository::open(&wt_path) {
            let statuses = wt_repo
                .statuses(None)
                .map_err(|e| format!("Failed to get status: {e}"))?;

            let has_changes = statuses.iter().any(|s| {
                let status = s.status();
                if status.is_empty() || status == git2::Status::IGNORED {
                    return false;
                }
                // Ignore our own metadata file
                if let Some(path) = s.path()
                    && path == ".archon-worktree.json" {
                        return false;
                    }
                true
            });

            if has_changes {
                return Err(format!(
                    "Worktree '{}' has uncommitted changes, refusing to clean up",
                    session_id
                ));
            }
        }

        // Remove the directory
        fs::remove_dir_all(&wt_path)
            .map_err(|e| format!("Failed to remove worktree directory: {e}"))?;

        Ok(())
    }
}

impl Default for WorktreeManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Resolve the data directory (`~/.local/share`).
fn dirs_next() -> PathBuf {
    if let Some(data) = dirs::data_dir() {
        data
    } else {
        // Fallback for systems without XDG
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".local").join("share")
    }
}

/// Convert a branch name like `archon/abc12345` to a valid worktree name.
fn branch_name_to_worktree_name(branch_name: &str) -> String {
    branch_name.replace('/', "-")
}

/// Prune a worktree reference from git.
fn prune_worktree(repo: &Repository, branch_name: &str) -> Result<(), String> {
    let wt_name = branch_name_to_worktree_name(branch_name);
    if let Ok(wt) = repo.find_worktree(&wt_name) {
        // Validate and prune if the worktree is prunable
        if wt.validate().is_err() {
            wt.prune(Some(
                git2::WorktreePruneOptions::new()
                    .valid(false)
                    .locked(false)
                    .working_tree(true),
            ))
            .map_err(|e| format!("Failed to prune worktree: {e}"))?;
        } else {
            wt.prune(Some(
                git2::WorktreePruneOptions::new()
                    .valid(true)
                    .locked(false)
                    .working_tree(true),
            ))
            .map_err(|e| format!("Failed to prune worktree: {e}"))?;
        }
    }
    Ok(())
}

/// Merge the worktree branch into the original branch.
fn merge_worktree_branch(repo: &Repository, info: &WorktreeInfo) -> Result<(), String> {
    // Get the commit on the worktree branch
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

    // Get the current HEAD commit on the original branch
    let head = repo
        .head()
        .map_err(|e| format!("Failed to get HEAD: {e}"))?;
    let head_commit = head
        .peel_to_commit()
        .map_err(|e| format!("Failed to peel HEAD to commit: {e}"))?;

    // Try fast-forward first
    if repo
        .graph_descendant_of(wt_commit_oid, head_commit.id())
        .unwrap_or(false)
    {
        // Fast-forward: just move HEAD to the worktree commit
        let refname = format!("refs/heads/{}", info.original_branch);
        repo.reference(
            &refname,
            wt_commit_oid,
            true,
            &format!("archon: fast-forward merge of {}", info.branch_name),
        )
        .map_err(|e| format!("Failed to fast-forward: {e}"))?;

        // Update HEAD to point to the new commit
        let obj = repo
            .find_object(wt_commit_oid, None)
            .map_err(|e| format!("Failed to find object: {e}"))?;
        repo.checkout_tree(&obj, None)
            .map_err(|e| format!("Failed to checkout after fast-forward: {e}"))?;
        repo.set_head(&refname)
            .map_err(|e| format!("Failed to set HEAD: {e}"))?;

        return Ok(());
    }

    // Non-fast-forward: create a merge commit
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

    // Create merge commit
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

    // Clean up merge state
    repo.cleanup_state()
        .map_err(|e| format!("Failed to cleanup state: {e}"))?;

    Ok(())
}
