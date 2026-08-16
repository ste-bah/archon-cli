use std::fs;
use std::path::Path;

use git2::{Repository, Signature};
use tempfile::TempDir;

use archon_tools::tool::Tool;
use archon_tools::worktree::EnterWorktreeTool;
use archon_tools::worktree::ExitWorktreeTool;
use archon_tools::worktree_manager::{ExitAction, WorktreeManager};
use archon_tools::worktree_ownership::{self, owner_key_for, session_owner_key};

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
///
/// `worktrees_dir()` is user-global and has no test override, so uniqueness of
/// the id — and therefore of the owner key derived from it — is the only thing
/// keeping these tests from racing each other or a developer's real session.
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
    let owner_id = session_owner_key(&session_id);

    let info =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("create worktree");

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
    let owner_id = session_owner_key(&session_id);

    let info =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("create worktree");

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
    let owner_id = session_owner_key(&session_id);

    let info =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("create worktree");

    // Branch should start with "archon/" and be derived from the OWNER key.
    assert!(
        info.branch_name.starts_with("archon/"),
        "branch should start with 'archon/', got: {}",
        info.branch_name
    );
    assert_eq!(info.branch_name, format!("archon/{owner_id}"));

    // Cleanup
    let _ = WorktreeManager::exit_worktree(&repo, &info, ExitAction::Discard);
}

#[test]
fn create_worktree_keeps_subagent_branches_unique() {
    let (_dir, repo) = init_repo_with_commit();
    // One session, two subagents: the branch must follow the owner key, not the
    // shared session id, or the second `repo.branch()` call collides.
    let session_id = unique_session_id("subuniq");
    let owner_one = "subagent-11111111-2222-3333-4444-555555555555";
    let owner_two = "subagent-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

    let info_one =
        WorktreeManager::create_worktree(&repo, &session_id, owner_one).expect("first worktree");
    let info_two =
        WorktreeManager::create_worktree(&repo, &session_id, owner_two).expect("second worktree");

    assert_ne!(info_one.branch_name, info_two.branch_name);
    assert_eq!(
        info_one.branch_name,
        "archon/subagent-11111111-2222-3333-4444-555555555555"
    );
    assert_eq!(
        info_two.branch_name,
        "archon/subagent-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
    );

    let _ = WorktreeManager::exit_worktree(&repo, &info_one, ExitAction::Discard);
    let _ = WorktreeManager::exit_worktree(&repo, &info_two, ExitAction::Discard);
}

#[test]
fn exit_keep_preserves_worktree() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("keep");
    let owner_id = session_owner_key(&session_id);

    let info =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("create worktree");

    let wt_path = info.worktree_path.clone();

    let result = WorktreeManager::exit_worktree(&repo, &info, ExitAction::Keep);
    assert!(result.is_ok(), "exit keep should succeed");

    assert!(
        wt_path.exists(),
        "worktree directory should still exist after 'keep'"
    );

    // Manual cleanup — `Keep` deliberately leaves the lock and marker in place.
    let _ = fs::remove_dir_all(&wt_path);
    worktree_ownership::forget(&WorktreeManager::worktrees_dir(), &owner_id);
}

#[test]
fn exit_discard_removes_worktree() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("discard");
    let owner_id = session_owner_key(&session_id);

    let info =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("create worktree");

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
    let owner_id = session_owner_key(&session_id);

    let info =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("create worktree");

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
    let owner_id = session_owner_key(&session_id);

    let info =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("create worktree");

    let list = WorktreeManager::list_worktrees();
    assert!(
        list.iter().any(|w| w.owner_id == owner_id),
        "list should contain owner_id '{}', got: {:?}",
        owner_id,
        list.iter().map(|w| &w.owner_id).collect::<Vec<_>>()
    );

    // Cleanup
    let _ = WorktreeManager::exit_worktree(&repo, &info, ExitAction::Discard);
}

#[test]
fn cleanup_removes_clean_worktree() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("cleanup");
    let owner_id = session_owner_key(&session_id);

    let info =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("create worktree");

    let wt_path = info.worktree_path.clone();

    // The parameter is the OWNER key, not the session id.
    let result = WorktreeManager::cleanup_session(&owner_id);
    assert!(result.is_ok(), "cleanup should succeed: {result:?}");

    assert!(
        !wt_path.exists(),
        "worktree directory should be removed after cleanup"
    );
}

// ---------------------------------------------------------------------------
// #184 M4 regression fixtures (worktree ownership / data loss)
// ---------------------------------------------------------------------------

/// Stand in for a worktree held by a *different* process.
///
/// A lock this process took is re-entrant by design (`owner_liveness` reports
/// `Ours`), so an in-process `acquire` cannot produce `Foreign`. Replacing the
/// lock file with a directory makes it unopenable, which is the same
/// "can't prove it's free, so treat it as held" branch a foreign holder takes.
fn simulate_foreign_holder(owner_id: &str) -> std::path::PathBuf {
    let root = WorktreeManager::worktrees_dir();
    worktree_ownership::release(owner_id);
    let lock = worktree_ownership::lock_path(&root, owner_id);
    let _ = fs::remove_file(&lock);
    fs::create_dir(&lock).expect("stand-in lock");
    lock
}

/// Guards: two agents in ONE session shared a directory, so the second agent's
/// `create_worktree` deleted the first's uncommitted work (#184 M4).
#[test]
fn two_agents_in_one_session_keep_separate_worktrees() {
    let (_dir, repo) = init_repo_with_commit();
    let suffix = unique_session_id("dual");
    let session_id = format!("same-session-{suffix}");
    let key_a = owner_key_for(&session_id, Some(&format!("agent-a-{suffix}")));
    let key_b = owner_key_for(&session_id, Some(&format!("agent-b-{suffix}")));
    assert_ne!(key_a, key_b);

    let a = WorktreeManager::create_worktree(&repo, &session_id, &key_a).expect("agent a worktree");
    let work = a.worktree_path.join("agent-a-work.txt");
    fs::write(&work, "uncommitted\n").expect("write agent a work");

    let b = WorktreeManager::create_worktree(&repo, &session_id, &key_b).expect("agent b worktree");

    assert_ne!(
        a.worktree_path, b.worktree_path,
        "each agent must get its own directory"
    );
    assert!(
        work.exists(),
        "agent a's uncommitted file must survive agent b's create_worktree"
    );

    // Refusal path: a directory held by someone else is never removed, and the
    // error names the owner so an operator knows who to wait for.
    let lock = simulate_foreign_holder(&key_a);

    let err = WorktreeManager::create_worktree(&repo, &session_id, &key_a)
        .expect_err("create must refuse a foreign-held worktree");
    assert!(err.contains(&key_a), "refusal must name the owner: {err}");

    let err = WorktreeManager::cleanup_session(&key_a)
        .expect_err("cleanup must refuse a foreign-held worktree");
    assert!(err.contains(&key_a), "refusal must name the owner: {err}");

    let _ = fs::remove_dir(&lock);
    let _ = WorktreeManager::exit_worktree(&repo, &a, ExitAction::Discard);
    let _ = WorktreeManager::exit_worktree(&repo, &b, ExitAction::Discard);
}

/// Guards the other half of the refusal: refusing must not be so cautious that
/// a genuinely abandoned directory is stranded forever. With no lock and no
/// accountable metadata, `create_worktree` reclaims it.
#[test]
fn an_abandoned_worktree_directory_is_reclaimed() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("reclaim");
    let owner_id = session_owner_key(&session_id);
    let root = WorktreeManager::worktrees_dir();

    let first =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("first worktree");
    let wt_path = first.worktree_path.clone();

    // Abandon it exactly as a killed process would: drop the lock and marker,
    // and remove the metadata that makes the directory accountable. Nothing
    // else is cleaned up — git still holds the `archon/<owner>` branch and the
    // worktree registration, and clearing those is the reclaim path's job.
    worktree_ownership::forget(&root, &owner_id);
    fs::remove_file(wt_path.join(".archon-worktree.json")).expect("remove metadata");
    fs::write(wt_path.join("leftover.txt"), "junk\n").expect("orphan content");

    let second = WorktreeManager::create_worktree(&repo, &session_id, &owner_id)
        .expect("abandoned worktree must be reclaimable");

    assert_eq!(second.worktree_path, wt_path);
    assert!(
        wt_path.is_dir(),
        "reclaim must leave a real worktree directory at {wt_path:?}"
    );
    assert!(
        !wt_path.join("leftover.txt").exists(),
        "reclaim should rebuild the directory, not adopt the orphan contents"
    );
    assert!(
        repo.find_branch(&second.branch_name, git2::BranchType::Local)
            .is_ok(),
        "reclaim must rebuild the branch '{}'",
        second.branch_name
    );

    let found = WorktreeManager::find_by_owner(&owner_id).expect("reclaimed worktree must resolve");
    assert_eq!(found.owner_id, owner_id);
    assert_eq!(found.worktree_path, wt_path);

    let _ = WorktreeManager::exit_worktree(&repo, &second, ExitAction::Discard);
}

/// Guards: exit resolved by session id, so a subagent looking up "its" worktree
/// got its PARENT's and then merged or discarded a tree it did not own.
#[test]
fn find_by_owner_returns_the_childs_worktree_not_the_parents() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("ownnotparent");
    let parent_key = owner_key_for(&session_id, None);
    let child_key = owner_key_for(&session_id, Some(&format!("child-1-{session_id}")));

    let parent =
        WorktreeManager::create_worktree(&repo, &session_id, &parent_key).expect("parent worktree");
    let child =
        WorktreeManager::create_worktree(&repo, &session_id, &child_key).expect("child worktree");

    let found = WorktreeManager::find_by_owner(&child_key).expect("child worktree must resolve");
    assert_eq!(found.owner_id, child_key);
    assert_eq!(found.worktree_path, child.worktree_path);
    assert_ne!(
        found.worktree_path, parent.worktree_path,
        "a child must never resolve to its parent's worktree"
    );

    let _ = WorktreeManager::exit_worktree(&repo, &child, ExitAction::Discard);
    let _ = WorktreeManager::exit_worktree(&repo, &parent, ExitAction::Discard);
}

/// Guards: an existing directory was assumed stale and deleted unconditionally,
/// so a resumed agent wiped its own previous run. Re-entry must reuse.
#[test]
fn re_entering_an_owned_worktree_reuses_it() {
    let (_dir, repo) = init_repo_with_commit();
    let session_id = unique_session_id("resume");
    let owner_id = session_owner_key(&session_id);

    let first =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("first worktree");
    let sentinel = first.worktree_path.join("resume-sentinel.txt");
    fs::write(&sentinel, "carried over\n").expect("write sentinel");

    let second =
        WorktreeManager::create_worktree(&repo, &session_id, &owner_id).expect("resume worktree");

    assert_eq!(
        second.worktree_path, first.worktree_path,
        "resume must return the same directory"
    );
    assert_eq!(second.owner_id, owner_id);
    assert!(
        sentinel.exists(),
        "resume must reuse the tree, not delete and recreate it"
    );

    let _ = WorktreeManager::exit_worktree(&repo, &second, ExitAction::Discard);
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
