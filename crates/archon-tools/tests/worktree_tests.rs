use std::fs;
use std::path::Path;

use git2::{Repository, Signature};
use tempfile::TempDir;

use archon_tools::tool::Tool;
use archon_tools::worktree::EnterWorktreeTool;
use archon_tools::worktree::ExitWorktreeTool;
use archon_tools::worktree_manager::{ExitAction, WorktreeManager};

/// Helper: create a temp dir with an initialized git repo and an initial commit.
fn init_repo_with_commit() -> (TempDir, Repository) {
    let dir = TempDir::new().expect("create temp dir");
    let repo = Repository::init(dir.path()).expect("git init");

    // Configure user for commits
    let mut config = repo.config().expect("repo config");
    config.set_str("user.name", "Test User").expect("set name");
    config
        .set_str("user.email", "test@example.com")
        .expect("set email");

    // Create an initial file and commit so HEAD exists
    let file_path = dir.path().join("README.md");
    fs::write(&file_path, "# Test Repo\n").expect("write readme");

    {
        let mut index = repo.index().expect("get index");
        index
            .add_path(Path::new("README.md"))
            .expect("add to index");
        index.write().expect("write index");

        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let sig = Signature::now("Test User", "test@example.com").expect("signature");
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .expect("initial commit");
    }

    (dir, repo)
}

/// Generate a unique session ID for test isolation.
fn unique_session_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    format!("{prefix}-{nanos}")
}

// ---------------------------------------------------------------------------
// WorktreeManager unit tests
// ---------------------------------------------------------------------------

#[test]
fn worktrees_dir_path() {
    let wt_dir = WorktreeManager::worktrees_dir();
    let path_str = wt_dir.to_string_lossy();
    assert!(
        path_str.contains("archon") && path_str.contains("worktrees"),
        "worktrees_dir should contain 'archon' and 'worktrees', got: {path_str}"
    );
}

#[test]
fn create_worktree_creates_directory() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("crdir");

    let info = WorktreeManager::create_worktree(&repo, &session_id).expect("create worktree");

    assert!(
        info.worktree_path.exists(),
        "worktree directory should exist at {:?}",
        info.worktree_path
    );

    // Cleanup
    let _ = WorktreeManager::exit_worktree(&repo, &info, ExitAction::Discard);
}

#[test]
fn create_worktree_creates_branch() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("crbr");

    let info = WorktreeManager::create_worktree(&repo, &session_id).expect("create worktree");

    // Verify the branch exists in the repo
    let branch = repo.find_branch(&info.branch_name, git2::BranchType::Local);
    assert!(
        branch.is_ok(),
        "branch '{}' should exist in repo",
        info.branch_name
    );

    // Cleanup
    let _ = WorktreeManager::exit_worktree(&repo, &info, ExitAction::Discard);
}

#[test]
fn create_worktree_branch_name() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("brname");

    let info = WorktreeManager::create_worktree(&repo, &session_id).expect("create worktree");

    // Branch should start with "archon/" and contain a shortened session id
    assert!(
        info.branch_name.starts_with("archon/"),
        "branch should start with 'archon/', got: {}",
        info.branch_name
    );
    // The short part is the first 8 chars of the session id
    let short = &session_id[..8.min(session_id.len())];
    assert!(
        info.branch_name.contains(short),
        "branch should contain session short '{}', got: {}",
        short,
        info.branch_name
    );

    // Cleanup
    let _ = WorktreeManager::exit_worktree(&repo, &info, ExitAction::Discard);
}

#[test]
fn exit_keep_preserves_worktree() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("keep");

    let info = WorktreeManager::create_worktree(&repo, &session_id).expect("create worktree");

    let wt_path = info.worktree_path.clone();

    let result = WorktreeManager::exit_worktree(&repo, &info, ExitAction::Keep);
    assert!(result.is_ok(), "exit keep should succeed");

    assert!(
        wt_path.exists(),
        "worktree directory should still exist after 'keep'"
    );

    // Manual cleanup
    let _ = fs::remove_dir_all(&wt_path);
}

#[test]
fn exit_discard_removes_worktree() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("discard");

    let info = WorktreeManager::create_worktree(&repo, &session_id).expect("create worktree");

    let wt_path = info.worktree_path.clone();

    let result = WorktreeManager::exit_worktree(&repo, &info, ExitAction::Discard);
    assert!(result.is_ok(), "exit discard should succeed");

    assert!(
        !wt_path.exists(),
        "worktree directory should be removed after 'discard'"
    );
}

#[test]
fn exit_merge_integrates_changes() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("merge");

    let info = WorktreeManager::create_worktree(&repo, &session_id).expect("create worktree");

    // Create a file in the worktree and commit it
    let new_file = info.worktree_path.join("merge_test.txt");
    fs::write(&new_file, "merge test content\n").expect("write file in worktree");

    // Open the worktree as a repo and commit
    let wt_repo = Repository::open(&info.worktree_path).expect("open worktree repo");
    {
        let mut index = wt_repo.index().expect("get index");
        index
            .add_path(Path::new("merge_test.txt"))
            .expect("add to index");
        index.write().expect("write index");

        let tree_id = index.write_tree().expect("write tree");
        let tree = wt_repo.find_tree(tree_id).expect("find tree");
        let sig = Signature::now("Test User", "test@example.com").expect("signature");
        let head = wt_repo.head().expect("head");
        let parent = head.peel_to_commit().expect("peel to commit");
        wt_repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "Add merge_test.txt",
                &tree,
                &[&parent],
            )
            .expect("commit in worktree");
    }

    // Now merge back
    let result = WorktreeManager::exit_worktree(&repo, &info, ExitAction::Merge);
    assert!(result.is_ok(), "exit merge should succeed: {:?}", result);

    // Check that the merge_test.txt is now accessible on the original branch
    let head = repo.head().expect("head");
    let commit = head.peel_to_commit().expect("peel to commit");
    let tree = commit.tree().expect("tree");
    let entry = tree.get_name("merge_test.txt");
    assert!(
        entry.is_some(),
        "merge_test.txt should be in the tree after merge"
    );
}

#[test]
fn list_worktrees_finds_created() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("list");

    let info = WorktreeManager::create_worktree(&repo, &session_id).expect("create worktree");

    let list = WorktreeManager::list_worktrees();
    assert!(
        list.iter().any(|w| w.session_id == session_id),
        "list should contain session_id '{}', got: {:?}",
        session_id,
        list.iter().map(|w| &w.session_id).collect::<Vec<_>>()
    );

    // Cleanup
    let _ = WorktreeManager::exit_worktree(&repo, &info, ExitAction::Discard);
}

#[test]
fn cleanup_removes_clean_worktree() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("cleanup");

    let info = WorktreeManager::create_worktree(&repo, &session_id).expect("create worktree");

    let wt_path = info.worktree_path.clone();

    let result = WorktreeManager::cleanup_session(&session_id);
    assert!(result.is_ok(), "cleanup should succeed");

    assert!(
        !wt_path.exists(),
        "worktree directory should be removed after cleanup"
    );
}

// ---------------------------------------------------------------------------
// Tool trait tests
// ---------------------------------------------------------------------------

#[test]
fn enter_worktree_tool_name() {
    let tool = EnterWorktreeTool;
    assert_eq!(tool.name(), "EnterWorktree");
}

#[test]
fn exit_worktree_tool_name() {
    let tool = ExitWorktreeTool;
    assert_eq!(tool.name(), "ExitWorktree");
}

// tools_registered_in_dispatch test is in archon-core/src/dispatch.rs
// because archon-tools cannot depend on archon-core (circular dependency)
