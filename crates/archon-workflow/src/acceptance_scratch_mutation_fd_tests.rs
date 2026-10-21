#![cfg(unix)]

use super::*;
use std::os::unix::fs::symlink;

#[test]
fn moves_and_restores_using_site_relative_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("site");
    std::fs::create_dir(&root_path).unwrap();
    std::fs::create_dir(root_path.join("input-dir")).unwrap();
    std::fs::write(root_path.join("input-dir/file"), b"original").unwrap();
    let root = SiteRoot::capture(&root_path).unwrap();

    assert_eq!(
        move_aside(
            &root,
            Path::new("input-dir/file"),
            Path::new("aside/run/file")
        ),
        MoveOutcome::Moved
    );
    assert!(!root_path.join("input-dir/file").exists());
    assert_eq!(
        restore(
            &root,
            Path::new("aside/run/file"),
            Path::new("input-dir/file")
        ),
        RestoreOutcome::Restored
    );
    assert_eq!(
        std::fs::read(root_path.join("input-dir/file")).unwrap(),
        b"original"
    );
}

#[test]
fn swapped_symlink_ancestor_is_rejected_without_touching_decoy() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("site");
    let decoy = temp.path().join("decoy");
    std::fs::create_dir_all(root_path.join("ancestor")).unwrap();
    std::fs::create_dir(&decoy).unwrap();
    std::fs::write(decoy.join("input"), b"decoy").unwrap();
    let root = SiteRoot::capture(&root_path).unwrap();

    std::fs::remove_dir(root_path.join("ancestor")).unwrap();
    symlink(&decoy, root_path.join("ancestor")).unwrap();
    assert!(matches!(
        restore(
            &root,
            Path::new("ancestor/aside"),
            Path::new("ancestor/input")
        ),
        RestoreOutcome::UnsafeToRestore(_)
    ));
    assert_eq!(std::fs::read(decoy.join("input")).unwrap(), b"decoy");
}

#[test]
fn replacing_site_root_keeps_mutations_on_the_held_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("site");
    std::fs::create_dir(&root_path).unwrap();
    std::fs::write(root_path.join("aside"), b"original").unwrap();
    let root = SiteRoot::capture(&root_path).unwrap();
    std::fs::rename(&root_path, temp.path().join("old-site")).unwrap();
    std::fs::create_dir(&root_path).unwrap();

    assert_eq!(
        restore(&root, Path::new("aside"), Path::new("target")),
        RestoreOutcome::Restored
    );
    assert!(temp.path().join("old-site/target").exists());
    assert!(!root_path.join("target").exists());
}

#[test]
fn renamed_root_ancestor_symlink_keeps_mutations_on_the_held_directory() {
    let temp = tempfile::tempdir().unwrap();
    let ancestor = temp.path().join("ancestor");
    let root_path = ancestor.join("site");
    let relocated = temp.path().join("relocated");
    std::fs::create_dir_all(root_path.join("data")).unwrap();
    std::fs::write(root_path.join("data/input"), b"original").unwrap();
    let outside = temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("input"), b"outside").unwrap();
    let root = SiteRoot::capture(&root_path).unwrap();

    std::fs::rename(&ancestor, &relocated).unwrap();
    symlink(&relocated, &ancestor).unwrap();

    assert_eq!(
        move_aside(&root, Path::new("data/input"), Path::new("aside/input")),
        MoveOutcome::Moved
    );
    assert!(!relocated.join("site/data/input").exists());
    assert_eq!(std::fs::read(outside.join("input")).unwrap(), b"outside");
    assert_eq!(
        restore(&root, Path::new("aside/input"), Path::new("data/input")),
        RestoreOutcome::Restored
    );
    assert_eq!(
        std::fs::read(relocated.join("site/data/input")).unwrap(),
        b"original"
    );
    assert_eq!(std::fs::read(outside.join("input")).unwrap(), b"outside");
}

#[test]
fn symlink_leaf_is_reported_without_moving_it() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("site");
    std::fs::create_dir(&root_path).unwrap();
    let decoy = temp.path().join("decoy");
    std::fs::write(&decoy, b"safe").unwrap();
    symlink(&decoy, root_path.join("input")).unwrap();
    let root = SiteRoot::capture(&root_path).unwrap();

    assert_eq!(
        move_aside(&root, Path::new("input"), Path::new("aside")),
        MoveOutcome::SymlinkComponent
    );
    assert!(
        root_path
            .join("input")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(decoy).unwrap(), b"safe");
}

#[test]
fn missing_input_and_aside_have_distinct_outcomes() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("site");
    std::fs::create_dir(&root_path).unwrap();
    let root = SiteRoot::capture(&root_path).unwrap();

    assert_eq!(
        move_aside(&root, Path::new("missing"), Path::new("aside")),
        MoveOutcome::Absent
    );
    assert_eq!(
        restore(&root, Path::new("missing-aside"), Path::new("target")),
        RestoreOutcome::AsideMissing
    );
}

#[test]
fn recreated_target_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("site");
    std::fs::create_dir(&root_path).unwrap();
    std::fs::write(root_path.join("aside"), b"original").unwrap();
    std::fs::write(root_path.join("target"), b"recreated").unwrap();
    let root = SiteRoot::capture(&root_path).unwrap();

    assert_eq!(
        restore(&root, Path::new("aside"), Path::new("target")),
        RestoreOutcome::OriginalRecreated
    );
    assert_eq!(std::fs::read(root_path.join("aside")).unwrap(), b"original");
    assert_eq!(
        std::fs::read(root_path.join("target")).unwrap(),
        b"recreated"
    );
}

#[test]
fn absolute_and_parent_paths_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("site");
    std::fs::create_dir(&root_path).unwrap();
    let root = SiteRoot::capture(&root_path).unwrap();

    assert!(matches!(
        move_aside(&root, Path::new("../outside"), Path::new("aside")),
        MoveOutcome::Failed(_)
    ));
    assert!(matches!(
        move_aside(&root, &temp.path().join("absolute"), Path::new("aside")),
        MoveOutcome::Failed(_)
    ));
    assert!(matches!(
        restore(&root, Path::new("aside"), Path::new("../outside")),
        RestoreOutcome::UnsafeToRestore(_)
    ));
}
