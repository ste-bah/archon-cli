use super::*;
use crate::workflow_read_guard::RunStoreScope;

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
    std::fs::create_dir_all(main.join("src")).expect("main");
    git(&main, &["init", "-q"]);
    git(&main, &["config", "user.email", "t@example.invalid"]);
    git(&main, &["config", "user.name", "t"]);
    std::fs::write(main.join("src/lib.txt"), "base\n").expect("write");
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

fn parent(dir: &Path, workflow: bool) -> ToolContext {
    ToolContext {
        working_dir: dir.to_path_buf(),
        run_store: workflow.then(RunStoreScope::default),
        ..ToolContext::default()
    }
}

#[test]
fn a_workflow_agent_placed_in_a_linked_worktree_seals_its_repository() {
    let (_t, main, a, b) = repository();
    let placement = Placement::resolve(&parent(&main, true), &a, None);
    assert_eq!(placement.working_dir(), a.as_path());
    let repo = placement
        .sealed_repository()
        .expect("isolated")
        .to_path_buf();
    let sealed = [repo];
    assert!(sealed_for(&main.join("src/lib.txt"), &a, &sealed));
    assert!(sealed_for(&b.join("x"), &a, &sealed), "a sibling is sealed");
    assert!(
        !sealed_for(&a.join("src/lib.txt"), &a, &sealed),
        "never its own"
    );
    // A worktree made after the spawn is covered without being listed.
    let late = main.parent().unwrap().join("wt-late");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "late",
            late.to_str().unwrap(),
        ],
    );
    assert!(sealed_for(&late.join("x"), &a, &sealed));
}

#[test]
fn an_interactive_agent_is_never_isolated() {
    let (_t, main, a, _b) = repository();
    assert_eq!(
        Placement::resolve(&parent(&main, false), &a, None),
        Placement::WorkingDir(a.clone())
    );
    assert_eq!(
        Placement::resolve(&parent(&main, false), &main, Some(&a)),
        Placement::WorkingDir(a.clone())
    );
    assert_eq!(
        Placement::child_dir(&parent(&a, false), None, &main),
        main,
        "outside a workflow the executor's directory is still the default"
    );
}

#[test]
fn an_agent_in_its_parents_checkout_or_a_plain_directory_shares_it() {
    let (_t, main, a, _b) = repository();
    assert_eq!(
        Placement::resolve(&parent(&main, true), &main.join("src"), None),
        Placement::WorkingDir(main.join("src"))
    );
    assert_eq!(
        Placement::resolve(&parent(&a, true), &a, None),
        Placement::WorkingDir(a.clone())
    );
    let plain = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        Placement::resolve(&parent(&main, true), plain.path(), None),
        Placement::WorkingDir(plain.path().to_path_buf())
    );
}

#[test]
fn a_worktree_the_spawn_made_seals_the_tree_it_was_made_from() {
    let (_t, main, a, _b) = repository();
    let placement = Placement::resolve(&parent(&main, true), &main, Some(&a));
    assert_eq!(placement.working_dir(), a.as_path());
    let sealed = [placement
        .sealed_repository()
        .expect("isolated")
        .to_path_buf()];
    assert!(sealed_for(&main.join("src/lib.txt"), &a, &sealed));
}

/// Issue-213 C3: a seal is inherited, never escaped by spawning. A child that
/// names no directory works where its parent does; one that names a sealed
/// checkout is placed in its parent's workspace.
#[test]
fn a_child_cannot_be_placed_outside_its_parents_seal() {
    let (_t, main, a, _b) = repository();
    let mut isolated = parent(&a, true);
    let placement = Placement::resolve(&parent(&main, true), &a, None);
    isolated.sealed_repositories = vec![placement.sealed_repository().unwrap().to_path_buf()];
    assert_eq!(Placement::child_dir(&isolated, None, &main), a);
    assert_eq!(Placement::child_dir(&isolated, Some(&main), &main), a);
    assert_eq!(
        Placement::child_dir(&isolated, Some(&a.join("src")), &main),
        a.join("src")
    );
}

#[test]
fn a_relative_or_empty_path_has_no_owning_checkout() {
    assert_eq!(owning_checkout(Path::new("")), None);
    assert_eq!(owning_checkout(Path::new("src/lib.rs")), None);
}

/// Issue-213 C3 (review): a submodule inside the canonical checkout belongs to
/// it, so it is sealed with it.
#[test]
fn a_submodule_inside_a_sealed_checkout_is_sealed_with_it() {
    let (_t, main, a, _b) = repository();
    let sub = main.join("vendor/sub");
    let modules = main.join(".git/modules/sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::create_dir_all(&modules).unwrap();
    std::fs::write(sub.join(".git"), format!("gitdir: {}\n", modules.display())).unwrap();
    let owner = owning_checkout(&sub.join("lib.c")).expect("owner");
    assert_eq!(owner.checkout, main);
    let sealed = [Placement::resolve(&parent(&main, true), &a, None)
        .sealed_repository()
        .unwrap()
        .to_path_buf()];
    assert!(sealed_for(&sub.join("lib.c"), &a, &sealed));
}

/// Issue-213 C3 (review): a `.git` file that is not a git link does not end
/// the walk; the checkout above it still owns the path.
#[test]
fn a_malformed_dot_git_file_does_not_hide_the_checkout_above() {
    let (_t, main, _a, _b) = repository();
    let odd = main.join("odd");
    std::fs::create_dir_all(&odd).unwrap();
    std::fs::write(odd.join(".git"), "not a git link\n").unwrap();
    assert_eq!(
        owning_checkout(&odd.join("x")).expect("owner").checkout,
        main
    );
}
