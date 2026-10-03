//! Issue 278: worktree cleanup deletes the agent's branch in the repository
//! the worktree belongs to, never through the process's current directory.
//!
//! The test binary runs with its package directory as the working directory
//! (`crates/archon-tools`), which is not the repository the worktree was made
//! from, so this is the "cwd is not the repository" case without touching the
//! process-global working directory.

use std::fs;
use std::path::Path;

use git2::{BranchType, Repository, Signature};
use tempfile::TempDir;

use archon_tools::worktree_manager::WorktreeManager;
use archon_tools::worktree_ownership::session_owner_key;

fn init_repo_with_commit() -> (TempDir, Repository) {
    let dir = TempDir::new().expect("create temp dir");
    let repo = Repository::init(dir.path()).expect("git init");
    fs::write(dir.path().join("README.md"), "# Test Repo\n").expect("write readme");
    {
        let mut index = repo.index().expect("get index");
        index.add_path(Path::new("README.md")).expect("add");
        index.write().expect("write index");
        let tree = repo
            .find_tree(index.write_tree().expect("write tree"))
            .expect("find tree");
        let sig = Signature::now("Test User", "test@example.com").expect("signature");
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .expect("initial commit");
    }
    (dir, repo)
}

#[test]
fn cleanup_deletes_the_branch_in_the_worktrees_own_repository() {
    let (_dir, repo) = init_repo_with_commit();
    let cwd = std::env::current_dir().expect("cwd");
    let cwd_repo = Repository::open(&cwd)
        .ok()
        .map(|r| r.commondir().to_path_buf());
    assert_ne!(
        cwd_repo.as_deref(),
        Some(repo.commondir()),
        "the test needs a working directory outside the worktree's repository"
    );
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let session_id = format!("cleanup-branch-{nanos}");
    let owner_id = session_owner_key(&session_id);
    let info =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("create worktree");
    assert!(
        repo.find_branch(&info.branch_name, BranchType::Local)
            .is_ok()
    );

    WorktreeManager::cleanup_session(&owner_id).expect("clean worktree cleans up");

    assert!(!info.worktree_path.exists(), "directory removed");
    assert!(
        repo.find_branch(&info.branch_name, BranchType::Local)
            .is_err(),
        "branch '{}' left behind in {}",
        info.branch_name,
        repo.path().display()
    );
    assert!(
        repo.worktrees().expect("worktrees").is_empty(),
        "worktree registration left behind"
    );
}
