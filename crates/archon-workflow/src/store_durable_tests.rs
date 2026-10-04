//! Durable publication on the host platform (on Windows, through the
//! write-through rename and the directory flush).

use super::*;

#[test]
fn a_write_replaces_its_target_whole_and_leaves_no_staging_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("nested").join("state.json");
    let tmp = dir.path().join("nested").join(".state.json.tmp");
    write_atomic(&tmp, &target, b"first").unwrap();
    write_atomic(&tmp, &target, b"second").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"second");
    assert!(!tmp.exists());
    sync_dir(target.parent().unwrap()).unwrap();
    sync_dir(dir.path()).unwrap();
}

#[test]
fn a_durable_rename_moves_a_file_and_a_missing_directory_does_not_sync() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a");
    std::fs::write(&from, b"kept").unwrap();
    let to = dir.path().join("b");
    rename_durable(&from, &to).unwrap();
    assert!(!from.exists());
    assert_eq!(std::fs::read(&to).unwrap(), b"kept");
    assert!(sync_dir(&dir.path().join("missing")).is_err());
}
