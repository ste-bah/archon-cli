//! Issue-234: the host-side snapshot boundary restores every change under a
//! sealed root and leaves the re-opened writable subtrees alone.

use super::*;

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn a_change_under_a_sealed_root_is_restored_and_named() {
    let dir = tempfile::tempdir().unwrap();
    let sealed = dir.path().join("checkout");
    let file = sealed.join("src/lib.rs");
    write(&file, b"original");
    let boundary = SnapshotBoundary::capture(std::slice::from_ref(&sealed), &[]);
    std::fs::write(&file, b"tampered").unwrap();
    let violation = boundary.verify_restore().expect_err("the change");
    // Named by its one canonical spelling.
    let named = crate::paths::plain(file.canonicalize().unwrap());
    assert!(violation.changed.contains(&named), "{violation:?}");
    assert_eq!(std::fs::read(&file).unwrap(), b"original");
}

/// The same directory reaches the boundary under several spellings (on
/// Windows an 8.3 short temp name, its long name and the verbatim form; on
/// macOS `/var` and `/private/var`). A worktree re-opened under one spelling
/// exempts its writes under a sealed root given in another.
#[test]
fn one_directory_under_two_spellings_is_one_directory() {
    let dir = tempfile::tempdir().unwrap();
    let sealed = dir.path().join("checkout");
    let worktree = sealed.join(".git/worktrees/item");
    write(&worktree.join("index"), b"before");
    write(&sealed.join("lib.rs"), b"base");
    let other_spelling = worktree.canonicalize().unwrap();
    let boundary = SnapshotBoundary::capture(
        std::slice::from_ref(&sealed),
        std::slice::from_ref(&other_spelling),
    );
    std::fs::write(worktree.join("index"), b"after").unwrap();
    boundary
        .verify_restore()
        .expect("the re-opened gitdir is the branch's");
    assert_eq!(std::fs::read(worktree.join("index")).unwrap(), b"after");
    std::fs::write(sealed.join("lib.rs"), b"tampered").unwrap();
    let boundary_again = SnapshotBoundary::capture(std::slice::from_ref(&other_spelling), &[]);
    std::fs::write(worktree.join("index"), b"again").unwrap();
    boundary_again
        .verify_restore()
        .expect_err("sealed under the other spelling");
    assert_eq!(std::fs::read(worktree.join("index")).unwrap(), b"after");
}

#[test]
fn a_created_file_under_a_sealed_root_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let sealed = dir.path().join("checkout");
    write(&sealed.join("keep"), b"x");
    let boundary = SnapshotBoundary::capture(std::slice::from_ref(&sealed), &[]);
    let forged = sealed.join("src/new.rs");
    write(&forged, b"forged");
    boundary.verify_restore().expect_err("the creation");
    assert!(!forged.exists(), "the forged file was not removed");
}

#[test]
fn a_removed_file_under_a_sealed_root_is_put_back() {
    let dir = tempfile::tempdir().unwrap();
    let sealed = dir.path().join("checkout");
    let file = sealed.join("data.json");
    write(&file, b"live");
    let boundary = SnapshotBoundary::capture(std::slice::from_ref(&sealed), &[]);
    std::fs::remove_file(&file).unwrap();
    boundary.verify_restore().expect_err("the removal");
    assert_eq!(std::fs::read(&file).unwrap(), b"live");
}

#[test]
fn a_write_inside_the_reopened_worktree_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let sealed = dir.path().join("project");
    let worktree = sealed.join(".archon/workflows/run1/wt");
    write(&sealed.join("src/lib.rs"), b"src");
    write(&worktree.join("own.txt"), b"before");
    let boundary = SnapshotBoundary::capture(
        std::slice::from_ref(&sealed),
        std::slice::from_ref(&worktree),
    );
    // The branch writes its own worktree: its work, not a violation.
    std::fs::write(worktree.join("own.txt"), b"after").unwrap();
    std::fs::write(worktree.join("extra.txt"), b"new").unwrap();
    boundary
        .verify_restore()
        .expect("the worktree is re-opened");
    assert_eq!(std::fs::read(worktree.join("own.txt")).unwrap(), b"after");
    assert_eq!(std::fs::read(worktree.join("extra.txt")).unwrap(), b"new");
}

#[test]
fn an_untouched_tree_is_no_violation() {
    let dir = tempfile::tempdir().unwrap();
    let sealed = dir.path().join("checkout");
    write(&sealed.join("a/b.rs"), b"x");
    write(&sealed.join("c.rs"), b"y");
    let boundary = SnapshotBoundary::capture(&[sealed], &[]);
    assert!(boundary.verify_restore().is_ok());
}

/// A writable directory that contains a sealed root never re-opens it, even
/// spelled differently from the root (the host temp directory beside a run).
#[test]
fn a_writable_directory_around_a_sealed_root_reopens_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let sealed = dir.path().canonicalize().unwrap().join("checkout");
    let file = sealed.join("lib.rs");
    write(&file, b"base");
    let around = dir.path().to_path_buf();
    let boundary = SnapshotBoundary::capture(std::slice::from_ref(&sealed), &[around]);
    std::fs::write(&file, b"tampered").unwrap();
    boundary
        .verify_restore()
        .expect_err("the sealed root stays sealed");
    assert_eq!(std::fs::read(&file).unwrap(), b"base");
}
