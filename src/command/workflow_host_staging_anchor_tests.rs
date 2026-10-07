//! The anchor's own edge cases (#297 round 7): links swapped in after the
//! creation check, links inside the tree, locked entries, FIFOs and depth.
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Tree {
    _temp: tempfile::TempDir,
    run: PathBuf,
    outside: PathBuf,
    anchor: StagingAnchor,
}

fn tree() -> Tree {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("run");
    std::fs::create_dir(&run).unwrap();
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(outside.join("call").join("locked")).unwrap();
    std::fs::write(outside.join("call").join("keep.txt"), b"outside").unwrap();
    mode(&outside.join("call").join("locked"), 0o555);
    let anchor = StagingAnchor::create(&run, "call").unwrap();
    std::fs::create_dir_all(anchor.root().join("nested").join("deep")).unwrap();
    std::fs::write(anchor.root().join("nested").join("deep").join("a"), b"a").unwrap();
    std::fs::write(anchor.root().join("gate-envelope.json"), b"{}").unwrap();
    Tree {
        _temp: temp,
        run,
        outside,
        anchor,
    }
}

fn mode(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn outside_intact(tree: &Tree) {
    let call = tree.outside.join("call");
    assert_eq!(std::fs::read(call.join("keep.txt")).unwrap(), b"outside");
    let locked = std::fs::metadata(call.join("locked")).unwrap();
    assert_eq!(locked.permissions().mode() & 0o777, 0o555, "never chmodded");
    mode(&call.join("locked"), 0o755);
}

#[test]
fn a_parent_swapped_for_a_link_after_creation_is_never_followed() {
    let tree = tree();
    let staging = tree.run.join(STAGING_DIR);
    std::fs::rename(&staging, tree.run.join("moved")).unwrap();
    symlink(&tree.outside, &staging).unwrap();
    assert!(
        tree.anchor.verify().is_err(),
        "the path no longer leads to it"
    );
    tree.anchor.remove_tree().unwrap();
    assert!(!tree.run.join("moved").join("call").exists());
    outside_intact(&tree);
}

#[test]
fn a_call_directory_swapped_for_a_link_is_unlinked_not_followed() {
    let tree = tree();
    std::fs::rename(tree.anchor.root(), tree.run.join("moved-call")).unwrap();
    symlink(tree.outside.join("call"), tree.anchor.root()).unwrap();
    assert!(
        tree.anchor.read_file("keep.txt").is_err(),
        "refused, not read"
    );
    assert!(tree.anchor.write_file("keep.txt", b"x").is_err());
    assert!(tree.anchor.scan(false, &mut |_, _| true).is_err());
    tree.anchor.remove_tree().unwrap();
    assert!(std::fs::symlink_metadata(tree.anchor.root()).is_err());
    outside_intact(&tree);
}

#[test]
fn links_inside_the_tree_are_removed_and_never_followed() {
    let tree = tree();
    symlink(
        tree.outside.join("call"),
        tree.anchor.root().join("nested").join("link"),
    )
    .unwrap();
    symlink(
        tree.outside.join("call").join("keep.txt"),
        tree.anchor.root().join("file-link"),
    )
    .unwrap();
    let mut seen = Vec::new();
    tree.anchor
        .scan(true, &mut |path, bytes| {
            seen.push((path.to_path_buf(), bytes.map(<[u8]>::to_vec)));
            false
        })
        .unwrap();
    seen.sort();
    assert_eq!(
        seen,
        vec![
            (PathBuf::from("gate-envelope.json"), Some(b"{}".to_vec())),
            (PathBuf::from("nested/deep/a"), Some(b"a".to_vec())),
        ],
        "only regular files inside the tree are read"
    );
    tree.anchor.remove_tree().unwrap();
    assert!(!tree.anchor.root().exists());
    outside_intact(&tree);
}

#[test]
fn locked_directories_and_files_are_removed() {
    let tree = tree();
    let nested = tree.anchor.root().join("nested");
    mode(&nested.join("deep").join("a"), 0);
    mode(&nested.join("deep"), 0);
    mode(&nested, 0o500);
    mode(tree.anchor.root(), 0);
    tree.anchor.remove_tree().unwrap();
    assert!(!tree.anchor.root().exists());
    outside_intact(&tree);
}

#[test]
fn a_fifo_envelope_never_blocks_and_is_replaced_by_a_regular_file() {
    let tree = tree();
    let envelope = tree.anchor.root().join("gate-envelope.json");
    std::fs::remove_file(&envelope).unwrap();
    let path = std::ffi::CString::new(envelope.to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert_eq!(tree.anchor.read_file("gate-envelope.json").unwrap(), None);
    tree.anchor
        .write_file("gate-envelope.json", b"sealed")
        .unwrap();
    let metadata = std::fs::symlink_metadata(&envelope).unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::read(&envelope).unwrap(), b"sealed");
    outside_intact(&tree);
}

#[test]
fn creation_removes_a_link_left_at_the_staging_root_and_never_follows_it() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("run");
    std::fs::create_dir(&run).unwrap();
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(outside.join("call")).unwrap();
    std::fs::write(outside.join("call").join("keep.txt"), b"outside").unwrap();
    symlink(&outside, run.join(STAGING_DIR)).unwrap();
    let anchor = StagingAnchor::create(&run, "call").unwrap();
    let staging = std::fs::symlink_metadata(run.join(STAGING_DIR)).unwrap();
    assert!(staging.is_dir() && !staging.file_type().is_symlink());
    anchor.verify().unwrap();
    assert_eq!(
        std::fs::read(outside.join("call").join("keep.txt")).unwrap(),
        b"outside"
    );
}

#[test]
fn a_tree_deeper_than_the_bound_is_an_error_not_a_crash() {
    let tree = tree();
    let mut deep = tree.anchor.root().to_path_buf();
    for _ in 0..300 {
        deep.push("d");
    }
    std::fs::create_dir_all(&deep).unwrap();
    let error = tree.anchor.remove_tree().unwrap_err();
    assert!(error.to_string().contains("deeper than"), "{error}");
    assert!(tree.anchor.scan(false, &mut |_, _| false).is_err());
    outside_intact(&tree);
}
