use super::*;

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "{args:?}: {out:?}");
}

/// A repository with one commit and two linked worktrees.
fn repository() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(temp.path()).expect("real temp");
    let main = root.join("main");
    std::fs::create_dir_all(&main).expect("main");
    git(&main, &["init", "-q"]);
    git(&main, &["config", "user.email", "t@example.invalid"]);
    git(&main, &["config", "user.name", "t"]);
    std::fs::write(main.join("f.txt"), "base\n").expect("write");
    git(&main, &["add", "."]);
    git(&main, &["commit", "-qm", "base"]);
    let a = root.join("wt-a");
    let b = root.join("wt-b");
    for (wt, branch) in [(&a, "a"), (&b, "b")] {
        git(
            &main,
            &["worktree", "add", "-q", "-b", branch, wt.to_str().unwrap()],
        );
    }
    (temp, main, a, b)
}

#[test]
fn an_agent_placed_in_a_linked_worktree_is_isolated_from_every_other_checkout() {
    let (_t, main, a, b) = repository();
    let placement = Placement::resolve(&main, Some(&a), None);
    assert_eq!(placement.working_dir(), a.as_path());
    let mut sealed = placement.sealed_roots().to_vec();
    sealed.sort();
    assert_eq!(sealed, vec![main, b]);
}

#[test]
fn an_agent_in_its_parents_directory_or_a_plain_directory_shares_it() {
    let (_t, main, a, _b) = repository();
    assert_eq!(
        Placement::resolve(&main, None, None),
        Placement::WorkingDir(main.clone())
    );
    // Even a linked worktree is shared when it IS the parent's directory.
    assert_eq!(
        Placement::resolve(&a, Some(&a), None),
        Placement::WorkingDir(a.clone())
    );
    let plain = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        Placement::resolve(&main, Some(plain.path()), None),
        Placement::WorkingDir(plain.path().to_path_buf())
    );
}

#[test]
fn a_worktree_the_spawn_made_seals_the_tree_it_was_made_from() {
    let (_t, main, a, b) = repository();
    let placement = Placement::resolve(&main, None, Some(&a));
    assert_eq!(placement.working_dir(), a.as_path());
    let sealed = placement.sealed_roots();
    assert!(sealed.contains(&main) && sealed.contains(&b), "{sealed:?}");
    assert!(!sealed.contains(&a), "its own workspace is never sealed");
    // A source git cannot list is still sealed.
    let plain = tempfile::tempdir().expect("tempdir");
    let placement = Placement::resolve(&main, Some(plain.path()), Some(&a));
    assert!(
        placement
            .sealed_roots()
            .contains(&plain.path().to_path_buf())
    );
}
