use super::*;

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "{args:?}");
}

/// A repository with one commit and a linked worktree on branch `wt`.
fn repo_with_worktree(root: &Path) -> (PathBuf, PathBuf) {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    git(&repo, &["config", "user.name", "fixture"]);
    std::fs::write(repo.join("seed"), "base").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);
    let worktree = root.join("wt");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt",
            worktree.to_str().unwrap(),
        ],
    );
    (repo, worktree)
}

#[test]
fn a_worktree_is_still_itself_and_nothing_else_is() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, worktree) = repo_with_worktree(temp.path());
    let identity = WorktreeIdentity::of(&worktree).unwrap();
    identity.check(&worktree).unwrap();
    assert!(
        WorktreeIdentity::of(&repo).is_err(),
        "the main checkout is not a linked worktree"
    );

    git(&worktree, &["switch", "-q", "-c", "other"]);
    assert!(
        identity.check(&worktree).is_err(),
        "a different branch was accepted"
    );
    git(&worktree, &["switch", "-q", "wt"]);

    std::fs::remove_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&worktree).unwrap();
    assert!(
        identity.check(&worktree).is_err(),
        "a recreated plain directory was accepted"
    );
}
