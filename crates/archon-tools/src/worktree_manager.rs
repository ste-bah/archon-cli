use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use git2::{BranchType, Repository};

use crate::worktree_exit::prune_worktree;
use crate::worktree_ownership;

/// Information about a single worktree session.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// The agent that owns this worktree — `session-<id>` or `subagent-<id>`.
    ///
    /// This, not `session_id`, names the directory. Every agent in a session
    /// shares one `session_id`, so keying on it made two agents fight over one
    /// folder and the loser's uncommitted work was deleted (#184 M4).
    pub owner_id: String,
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

    /// Create — or reuse — the worktree owned by `owner_id`.
    ///
    /// The directory is `<worktrees_dir>/<owner_id>/`, named after the **agent**
    /// rather than the session. Before this, it was named after the session,
    /// and an existing directory was assumed stale and deleted unconditionally;
    /// since every agent in a session shares one session id, a second agent
    /// destroyed the first's uncommitted work (#184 M4).
    ///
    /// Three outcomes, in order:
    ///
    /// - **Owned by someone else** — refuse, naming the owner. A live agent's
    ///   tree is never deleted, whatever the caller intended.
    /// - **Ours and intact** — reuse it. A resumed agent wants its files still
    ///   there; deleting and recreating would throw away the continuity that
    ///   transcript resume exists to preserve.
    /// - **Abandoned or broken** — reclaim: drop the stale lock and marker,
    ///   remove the directory, and build a fresh worktree.
    pub fn create_worktree(
        repo: &Repository,
        session_id: &str,
        owner_id: &str,
    ) -> Result<WorktreeInfo, String> {
        let root = Self::worktrees_dir();
        let wt_dir = root.join(owner_id);

        fs::create_dir_all(&root)
            .map_err(|e| format!("Failed to create worktrees parent directory: {e}"))?;

        if wt_dir.exists() {
            if worktree_ownership::owner_liveness(&root, owner_id)
                == worktree_ownership::OwnerLiveness::Foreign
            {
                return Err(format!(
                    "Worktree '{}' is in use by {} — refusing to remove it. \
                     Wait for that agent to finish, or use a different agent identity.",
                    owner_id,
                    worktree_ownership::describe_owner(&root, owner_id),
                ));
            }

            if let Some(existing) = read_worktree_meta(&wt_dir) {
                worktree_ownership::acquire(&root, owner_id)?;
                return Ok(existing);
            }

            // Present but not a worktree we can account for: genuinely stale.
            //
            // The directory is only half of it. git still holds a worktree
            // registration and an `archon/<owner>` branch pointing at the old
            // tree, and creating the replacement fails with "a reference with
            // that name already exists" if they are left behind — so an
            // abandoned worktree could be detected but never actually
            // reclaimed, which is the whole point of this branch. Best-effort
            // throughout: a missing registration or branch is the state we
            // want, not an error.
            worktree_ownership::forget(&root, owner_id);
            fs::remove_dir_all(&wt_dir)
                .map_err(|e| format!("Failed to remove stale worktree directory: {e}"))?;

            let stale_branch = format!("archon/{}", branch_component_from_session_id(owner_id));
            let _ = prune_worktree(repo, &stale_branch);
            if let Ok(mut branch) = repo.find_branch(&stale_branch, BranchType::Local) {
                let _ = branch.delete();
            }
        }

        worktree_ownership::acquire(&root, owner_id)?;

        // Release the lock if the build fails partway. Otherwise a half-created
        // worktree keeps a lock naming a directory that does not exist, and the
        // retry sees its own stale hold instead of a clean slate.
        let built = Self::build_worktree(repo, session_id, owner_id, &root, &wt_dir);
        if built.is_err() {
            worktree_ownership::release(owner_id);
        }
        built
    }

    /// Branch, worktree, metadata and marker for a directory already cleared
    /// for use. Split out so [`create_worktree`] can release the ownership lock
    /// on any failure in here without threading a guard through every `?`.
    fn build_worktree(
        repo: &Repository,
        session_id: &str,
        owner_id: &str,
        root: &std::path::Path,
        wt_dir: &std::path::Path,
    ) -> Result<WorktreeInfo, String> {
        let original_branch = crate::git::current_branch(repo)?;
        let original_dir = repo
            .workdir()
            .ok_or_else(|| "Repository has no workdir (bare repo)".to_string())?
            .to_path_buf();

        // Derived from the owner, not the session: two agents in one session
        // would otherwise ask git for the same branch and the second would fail.
        let branch_name = format!("archon/{}", branch_component_from_session_id(owner_id));

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
            wt_dir,
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
            "owner_id": owner_id,
            "session_id": session_id,
            "branch_name": branch_name,
            "original_dir": original_dir.to_string_lossy(),
            "original_branch": original_branch,
            "created_at": now.to_rfc3339(),
        });
        fs::write(&meta_path, meta.to_string())
            .map_err(|e| format!("Failed to write worktree metadata: {e}"))?;

        // Identity marker beside the lock, so a later refusal can say who owns
        // this directory instead of only that someone does.
        worktree_ownership::write_marker(root, owner_id, session_id, &now.to_rfc3339())?;

        Ok(WorktreeInfo {
            owner_id: owner_id.to_string(),
            session_id: session_id.to_string(),
            branch_name,
            worktree_path: wt_dir.to_path_buf(),
            original_dir,
            original_branch,
            created_at: now,
        })
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
            if let Some(info) = read_worktree_meta(&path) {
                result.push(info);
            }
        }

        result
    }

    /// The worktree owned by `owner_id`, if one exists.
    ///
    /// Exists so callers resolve by **owner** rather than by session. Matching
    /// on `session_id` returned the parent's worktree to a subagent, which then
    /// merged or discarded a tree it did not own (#184 M4).
    pub fn find_by_owner(owner_id: &str) -> Option<WorktreeInfo> {
        let wt_dir = Self::worktrees_dir().join(owner_id);
        if !wt_dir.is_dir() {
            return None;
        }
        read_worktree_meta(&wt_dir)
    }

    /// Convenience wrapper: open repo at `working_dir` and create a worktree.
    pub fn create_worktree_from_path(
        working_dir: &std::path::Path,
        session_id: &str,
        owner_id: &str,
    ) -> Result<WorktreeInfo, String> {
        let repo = Repository::open(working_dir).map_err(|e| {
            format!(
                "Failed to open repository at {}: {e}",
                working_dir.display()
            )
        })?;
        Self::create_worktree(&repo, session_id, owner_id)
    }

    /// Cleanup a worktree session if it has no uncommitted changes.
    /// Remove the worktree owned by `owner_id`, if it is safe to.
    ///
    /// Refuses while another process holds the ownership lock: cleanup runs on
    /// completion paths that can fire while a peer is still writing, and this
    /// is the last thing standing between a live agent and `remove_dir_all`.
    pub fn cleanup_session(owner_id: &str) -> Result<(), String> {
        let root = Self::worktrees_dir();
        let wt_path = root.join(owner_id);
        if !wt_path.exists() {
            worktree_ownership::forget(&root, owner_id);
            return Ok(());
        }

        if worktree_ownership::owner_liveness(&root, owner_id)
            == worktree_ownership::OwnerLiveness::Foreign
        {
            return Err(format!(
                "Worktree '{}' is in use by {} — refusing to clean it up",
                owner_id,
                worktree_ownership::describe_owner(&root, owner_id),
            ));
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
                    && path == ".archon-worktree.json"
                {
                    return false;
                }
                true
            });

            if has_changes {
                return Err(format!(
                    "Worktree '{}' has uncommitted changes, refusing to clean up",
                    owner_id
                ));
            }
        }

        // The scratch build directory first, while its path is still derivable
        // and before anything can fail. A pruned worktree that leaves gigabytes
        // of `target/` behind has not been pruned (#184 M3).
        let scratch = Self::scratch_target_dir(owner_id);
        if scratch.exists() {
            fs::remove_dir_all(&scratch)
                .map_err(|e| format!("Failed to remove worktree build directory: {e}"))?;
        }

        // Remove the directory
        fs::remove_dir_all(&wt_path)
            .map_err(|e| format!("Failed to remove worktree directory: {e}"))?;

        // git still holds a worktree registration and an `archon/<owner>`
        // branch. Leaving them made the directory removable but the worktree
        // unreclaimable: the replacement failed with "a reference with that
        // name already exists". Best-effort, because a missing one is the state
        // we want.
        let branch_name = format!("archon/{}", branch_component_from_session_id(owner_id));
        if let Ok(repo) = Repository::open(&wt_path).or_else(|_| Repository::open(".")) {
            let _ = prune_worktree(&repo, &branch_name);
            if let Ok(mut branch) = repo.find_branch(&branch_name, BranchType::Local) {
                let _ = branch.delete();
            }
        }

        // Release the lock and drop the marker only once the tree is actually
        // gone, so a failure above leaves the ownership record intact.
        worktree_ownership::forget(&root, owner_id);

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
///
/// The last-resort branch used to hardcode `/tmp` (issue #156). On Windows
/// that is not a temp directory: it resolves against the current drive root,
/// so a worktree base of `F:\tmp\.local\share\archon\worktrees` would be
/// created at the root of whichever drive the process was launched from.
/// `std::env::temp_dir()` is the portable equivalent — `$TMPDIR` on unix,
/// `%TEMP%` on Windows.
fn dirs_next() -> PathBuf {
    if let Some(data) = dirs::data_dir() {
        data
    } else {
        // Fallback for systems without XDG
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        home.join(".local").join("share")
    }
}

/// Read `.archon-worktree.json` out of an existing worktree directory.
///
/// `None` means "this directory is not an archon worktree we can account for",
/// which is what licenses a reclaim. A directory that *does* parse belongs to
/// its recorded owner and is reused rather than rebuilt.
///
/// `owner_id` is defaulted from the directory name for worktrees written
/// before the field existed, so an upgrade reuses them instead of deleting
/// them on first contact.
fn read_worktree_meta(wt_dir: &std::path::Path) -> Option<WorktreeInfo> {
    let text = fs::read_to_string(wt_dir.join(".archon-worktree.json")).ok()?;
    let meta: serde_json::Value = serde_json::from_str(&text).ok()?;

    let fallback_owner = wt_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    Some(WorktreeInfo {
        owner_id: meta["owner_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .unwrap_or(fallback_owner)
            .to_string(),
        session_id: meta["session_id"].as_str().unwrap_or_default().to_string(),
        branch_name: meta["branch_name"].as_str().unwrap_or_default().to_string(),
        worktree_path: wt_dir.to_path_buf(),
        original_dir: PathBuf::from(meta["original_dir"].as_str().unwrap_or_default()),
        original_branch: meta["original_branch"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        created_at: meta["created_at"]
            .as_str()
            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
            .unwrap_or_else(Utc::now),
    })
}

/// Convert a branch name like `archon/abc12345` to a valid worktree name.
pub(crate) fn branch_name_to_worktree_name(branch_name: &str) -> String {
    branch_name.replace('/', "-")
}

fn branch_component_from_session_id(session_id: &str) -> String {
    const MAX_COMPONENT_LEN: usize = 64;

    let mut component = String::with_capacity(session_id.len().min(MAX_COMPONENT_LEN));
    let mut previous_was_dot = false;

    for ch in session_id.chars() {
        let mapped = match ch {
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => c,
            '.' => '.',
            _ => '-',
        };

        if mapped == '.' && previous_was_dot {
            continue;
        }

        component.push(mapped);
        previous_was_dot = mapped == '.';

        if component.len() >= MAX_COMPONENT_LEN {
            break;
        }
    }

    let mut component = component.trim_matches(|c| c == '-' || c == '.').to_string();
    while component.ends_with(".lock") {
        component.truncate(component.len() - ".lock".len());
        component = component.trim_matches(|c| c == '-' || c == '.').to_string();
    }

    if component.is_empty() {
        "session".to_string()
    } else {
        component
    }
}
