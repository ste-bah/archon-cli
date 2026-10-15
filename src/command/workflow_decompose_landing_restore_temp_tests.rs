use super::super::settle::restore_temp::prepare_restore_temp;
use super::super::settle::restore_temp::{RESTORE_TEMP_FAILURE, RestoreTempFailure};

#[test]
fn preparation_failure_removes_temp_and_preserves_live_file() {
    for failure in [RestoreTempFailure::Write, RestoreTempFailure::Sync] {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("task.md");
        std::fs::write(&live, b"live task bytes").unwrap();
        *RESTORE_TEMP_FAILURE.lock().unwrap() = Some(failure);

        let result = prepare_restore_temp(&live, b"restored bytes");

        assert!(result.is_err(), "injected preparation failure must fail");
        assert_eq!(std::fs::read(&live).unwrap(), b"live task bytes");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains(".restore-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporary files remain: {leftovers:?}"
        );
    }
}

#[test]
fn cleanup_guard_does_not_remove_file_recreated_after_successful_rename() {
    let dir = tempfile::tempdir().unwrap();
    let live = dir.path().join("task.md");
    let (temp, mut cleanup) = prepare_restore_temp(&live, b"prepared bytes").unwrap();

    std::fs::rename(&temp, &live).unwrap();
    cleanup.disarm();
    std::fs::write(&temp, b"new unrelated file").unwrap();
    drop(cleanup);

    assert_eq!(std::fs::read(&live).unwrap(), b"prepared bytes");
    assert_eq!(std::fs::read(&temp).unwrap(), b"new unrelated file");
}
