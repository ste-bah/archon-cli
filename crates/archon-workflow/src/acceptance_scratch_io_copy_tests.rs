use super::*;

#[cfg(test)]
#[test]
fn task_root_copy_omits_freeze_owned_contract_and_lock() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let contract = crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
    let lock = crate::task_set_contract::ACCEPTANCE_LOCK_FILE;
    std::fs::write(source.path().join(contract), b"contract").unwrap();
    std::fs::write(source.path().join(lock), b"lock").unwrap();
    std::fs::write(source.path().join("input.json"), b"input").unwrap();
    std::fs::write(
        source
            .path()
            .join(crate::task_set_contract::TASK_SKELETON_FILE),
        b"skeleton",
    )
    .unwrap();
    std::fs::write(
        source
            .path()
            .join(crate::task_set_contract::TASK_SKELETON_LOCK_FILE),
        b"skeleton lock",
    )
    .unwrap();
    std::fs::write(source.path().join("TASK-EXAMPLE-001.md"), b"landed body").unwrap();
    std::fs::write(source.path().join(".decompose.log"), b"pipeline log").unwrap();

    let mut remaining = u64::MAX;
    copy_task_root(
        source.path(),
        destination.path(),
        Path::new(""),
        Path::new(""),
        (&[], None),
        &mut remaining,
    )
    .unwrap();

    assert!(!destination.path().join(contract).exists());
    assert!(!destination.path().join(lock).exists());
    assert!(
        !destination
            .path()
            .join(crate::task_set_contract::TASK_SKELETON_FILE)
            .exists()
    );
    assert!(
        !destination
            .path()
            .join(crate::task_set_contract::TASK_SKELETON_LOCK_FILE)
            .exists()
    );
    assert!(!destination.path().join("TASK-EXAMPLE-001.md").exists());
    assert!(!destination.path().join(".decompose.log").exists());
    assert!(std::fs::read(destination.path().join("acceptance-contract.json")).is_err());
    assert_eq!(
        std::fs::read(destination.path().join("input.json")).unwrap(),
        b"input"
    );
}
