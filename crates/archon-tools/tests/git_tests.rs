use std::fs;
use std::path::Path;

use git2::{Repository, Signature};
use tempfile::TempDir;

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

/// Helper: create a bare temp dir (no git repo).
fn bare_temp_dir() -> TempDir {
    TempDir::new().expect("create temp dir")
}

// ---------------------------------------------------------------------------
// open_repo / current_branch
// ---------------------------------------------------------------------------

#[test]
fn open_repo_works() {
    let (dir, _repo) = init_repo_with_commit();
    let result = archon_tools::git::open_repo(dir.path());
    assert!(result.is_ok(), "should open a valid git repo");
}

#[test]
fn open_repo_non_git_fails() {
    let dir = bare_temp_dir();
    let result = archon_tools::git::open_repo(dir.path());
    assert!(result.is_err(), "should fail on non-git directory");
}

#[test]
fn current_branch_is_main() {
    let (_dir, repo) = init_repo_with_commit();
    let branch = archon_tools::git::current_branch(&repo);
    assert!(branch.is_ok());
    let name = branch.expect("branch name");
    // git2 defaults to "master" unless configured otherwise
    assert!(
        name == "main" || name == "master",
        "expected main or master, got: {name}"
    );
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

#[test]
fn git_status_clean_repo() {
    let (_dir, repo) = init_repo_with_commit();
    let info = archon_tools::git::status::git_status(&repo).expect("status");
    assert!(info.modified.is_empty(), "no modified files expected");
    assert!(info.staged.is_empty(), "no staged files expected");
    assert!(info.untracked.is_empty(), "no untracked files expected");
}

#[test]
fn git_status_modified_file() {
    let (dir, repo) = init_repo_with_commit();
    fs::write(dir.path().join("README.md"), "# Changed\n").expect("modify file");

    let info = archon_tools::git::status::git_status(&repo).expect("status");
    assert!(
        info.modified.contains(&"README.md".to_string()),
        "README.md should appear in modified, got: {:?}",
        info.modified
    );
}

#[test]
fn git_status_staged_file() {
    let (dir, repo) = init_repo_with_commit();
    fs::write(dir.path().join("README.md"), "# Staged change\n").expect("modify");

    // Stage the change
    let mut index = repo.index().expect("index");
    index
        .add_path(Path::new("README.md"))
        .expect("add to index");
    index.write().expect("write index");

    let info = archon_tools::git::status::git_status(&repo).expect("status");
    assert!(
        info.staged.contains(&"README.md".to_string()),
        "README.md should appear in staged, got: {:?}",
        info.staged
    );
}

#[test]
fn git_status_untracked_file() {
    let (dir, repo) = init_repo_with_commit();
    fs::write(dir.path().join("new_file.txt"), "untracked content").expect("create file");

    let info = archon_tools::git::status::git_status(&repo).expect("status");
    assert!(
        info.untracked.contains(&"new_file.txt".to_string()),
        "new_file.txt should appear in untracked, got: {:?}",
        info.untracked
    );
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

#[test]
fn git_diff_empty_on_clean() {
    let (_dir, repo) = init_repo_with_commit();
    let diff = archon_tools::git::diff::git_diff(&repo, false).expect("diff");
    assert!(diff.is_empty(), "clean repo should have empty diff");
}

#[test]
fn git_diff_shows_changes() {
    let (dir, repo) = init_repo_with_commit();
    fs::write(dir.path().join("README.md"), "# Modified content\n").expect("modify");

    let diff = archon_tools::git::diff::git_diff(&repo, false).expect("diff");
    assert!(
        diff.contains("README.md"),
        "diff should mention README.md, got: {diff}"
    );
}

#[test]
fn diff_stats_counts() {
    let (dir, repo) = init_repo_with_commit();
    fs::write(dir.path().join("README.md"), "# Modified\nNew line\n").expect("modify");

    let stats = archon_tools::git::diff::git_diff_stats(&repo).expect("diff stats");
    assert!(
        stats.files_changed >= 1,
        "at least one file changed, got: {}",
        stats.files_changed
    );
}

// ---------------------------------------------------------------------------
// branch
// ---------------------------------------------------------------------------

#[test]
fn list_branches_includes_current() {
    let (_dir, repo) = init_repo_with_commit();
    let branches = archon_tools::git::branch::list_branches(&repo).expect("list branches");
    assert!(!branches.is_empty(), "should have at least one branch");
    assert!(
        branches.iter().any(|b| b.is_current),
        "one branch should be current"
    );
}

#[test]
fn create_branch() {
    let (_dir, repo) = init_repo_with_commit();
    archon_tools::git::branch::create_branch(&repo, "feature").expect("create branch");

    let branches = archon_tools::git::branch::list_branches(&repo).expect("list branches");
    assert!(
        branches.iter().any(|b| b.name == "feature"),
        "feature branch should exist in: {:?}",
        branches.iter().map(|b| &b.name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// commit
// ---------------------------------------------------------------------------

#[test]
fn commit_creates_commit() {
    let (dir, repo) = init_repo_with_commit();
    // Create a new file and stage it
    fs::write(dir.path().join("new.txt"), "content").expect("create file");
    archon_tools::git::commit::stage_all(&repo).expect("stage all");

    let hash = archon_tools::git::commit::commit(&repo, "Add new.txt").expect("commit");
    assert!(!hash.is_empty(), "commit should return a non-empty hash");
    // Verify it's a valid hex string (at least 7 chars)
    assert!(hash.len() >= 7, "hash should be at least 7 chars: {hash}");
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "hash should be hex: {hash}"
    );
}

// ---------------------------------------------------------------------------
// format_status
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// PR module
// ---------------------------------------------------------------------------

#[test]
fn pr_build_command_with_title_and_body() {
    let cmd = archon_tools::git::pr::build_gh_command("Fix bug", Some("Detailed description"));
    assert_eq!(cmd[0], "gh");
    assert_eq!(cmd[1], "pr");
    assert_eq!(cmd[2], "create");
    assert!(cmd.contains(&"--title".to_string()));
    assert!(cmd.contains(&"Fix bug".to_string()));
    assert!(cmd.contains(&"--body".to_string()));
    assert!(cmd.contains(&"Detailed description".to_string()));
}

#[test]
fn pr_build_command_title_only() {
    let cmd = archon_tools::git::pr::build_gh_command("My PR", None);
    assert!(cmd.contains(&"--title".to_string()));
    assert!(cmd.contains(&"My PR".to_string()));
    // Should not have --body when body is None
    assert!(!cmd.contains(&"--body".to_string()));
}

#[test]
fn format_status_readable() {
    let info = archon_tools::git::status::GitStatusInfo {
        modified: vec!["file1.rs".to_string()],
        staged: vec!["file2.rs".to_string()],
        untracked: vec!["file3.rs".to_string()],
        branch: "main".to_string(),
    };
    let output = archon_tools::git::status::format_status(&info);
    assert!(output.contains("file1.rs"), "should contain modified file");
    assert!(output.contains("file2.rs"), "should contain staged file");
    assert!(output.contains("file3.rs"), "should contain untracked file");
    assert!(output.contains("main"), "should contain branch name");
}
