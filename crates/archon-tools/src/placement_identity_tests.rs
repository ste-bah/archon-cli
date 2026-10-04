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
    let identity = PlacementIdentity::of(&worktree).unwrap();
    identity.check(&worktree).unwrap();
    assert_ne!(
        PlacementIdentity::of(&repo).unwrap(),
        identity,
        "the main checkout is a different placement"
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

#[test]
fn a_moved_worktree_cannot_be_impersonated_by_a_pointer_left_at_its_old_path() {
    let temp = tempfile::tempdir().unwrap();
    let (_repo, worktree) = repo_with_worktree(temp.path());
    let identity = PlacementIdentity::of(&worktree).unwrap();
    let pointer = std::fs::read_to_string(worktree.join(".git")).unwrap();
    std::fs::rename(&worktree, temp.path().join("moved")).unwrap();
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::write(worktree.join(".git"), pointer).unwrap();
    assert!(
        identity.check(&worktree).is_err(),
        "a replacement pointing at the same administrative directory was accepted"
    );
}

#[test]
fn a_worktree_registered_again_under_the_same_name_and_branch_is_not_the_same() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, worktree) = repo_with_worktree(temp.path());
    let identity = PlacementIdentity::of(&worktree).unwrap();
    let path = worktree.to_str().unwrap();
    git(&repo, &["worktree", "remove", "--force", path]);
    git(&repo, &["worktree", "add", "-q", "-B", "wt", path]);
    assert!(
        identity.check(&worktree).is_err(),
        "a new registration with the same name and branch was accepted"
    );
}

#[test]
fn a_plain_directory_recreated_at_the_same_path_is_not_the_same() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("workspace");
    std::fs::create_dir_all(&dir).unwrap();
    let identity = PlacementIdentity::of(&dir).expect("a plain directory has an identity too");
    identity.check(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    assert!(
        identity.check(&dir).is_err(),
        "a recreated directory was accepted"
    );
}
