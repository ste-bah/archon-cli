use super::*;
use std::os::windows::fs::symlink_dir;

#[test]
fn ancestor_symlink_is_refused_before_moving_an_external_input() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("site");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(root_path.join("data")).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("input"), b"outside").unwrap();
    let root = SiteRoot::capture(&root_path).unwrap();

    std::fs::remove_dir(root_path.join("data")).unwrap();
    symlink_dir(&outside, root_path.join("data")).unwrap();
    assert!(matches!(
        move_aside(&root, Path::new("data/input"), Path::new("aside/input")),
        MoveOutcome::Failed(_) | MoveOutcome::SymlinkComponent
    ));
    assert_eq!(std::fs::read(outside.join("input")).unwrap(), b"outside");
}

#[test]
fn ordinary_directory_with_replaced_root_path_is_unsafe_to_restore() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("site");
    let replacement = temp.path().join("replacement");
    std::fs::create_dir(&root_path).unwrap();
    std::fs::write(root_path.join("aside"), b"original").unwrap();
    std::fs::create_dir(&replacement).unwrap();
    let mut root = SiteRoot::capture(&root_path).unwrap();
    root.path = replacement.clone();

    assert!(matches!(
        restore(&root, Path::new("aside"), Path::new("target")),
        RestoreOutcome::UnsafeToRestore(_)
    ));
    assert!(!replacement.join("target").exists());
    assert_eq!(std::fs::read(root_path.join("aside")).unwrap(), b"original");
}
