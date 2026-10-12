use super::*;

fn repository(source: &tempfile::TempDir) -> PathBuf {
    let path = source.path().join("repository");
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn freeze_outputs_are_absent_from_the_probe_copy_and_digest() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path().canonicalize().unwrap();
    let task = source.path().join("tasks/set");
    std::fs::create_dir_all(&task).unwrap();
    let contract = archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
    let lock = archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE;
    std::fs::write(task.join(contract), b"published contract").unwrap();
    std::fs::write(task.join(lock), b"published lock").unwrap();
    let same_named = source.path().join("config/acceptance-contract.json");
    std::fs::create_dir_all(same_named.parent().unwrap()).unwrap();
    std::fs::write(&same_named, b"project data").unwrap();
    let pin = source.path().join(".archon/task-set-pins/set.json");
    std::fs::create_dir_all(pin.parent().unwrap()).unwrap();
    std::fs::write(&pin, b"published pin").unwrap();
    std::fs::write(task.join("input.json"), b"stable input").unwrap();
    let task = task.canonicalize().unwrap();
    let before = data_digest(source.path(), &repository(&source), Some(&task)).unwrap();
    let mut remaining = u64::MAX;
    assert!(
        copy_data(
            &project,
            Path::new(""),
            destination.path(),
            &repository(&source),
            Some(&task),
            &mut remaining
        )
        .is_ok()
    );
    assert!(!destination.path().join("tasks/set").join(contract).exists());
    assert!(!destination.path().join("tasks/set").join(lock).exists());
    assert!(
        !destination
            .path()
            .join(".archon/task-set-pins/set.json")
            .exists()
    );
    assert_eq!(
        before,
        data_digest(source.path(), &repository(&source), Some(&task)).unwrap()
    );
    assert_eq!(
        std::fs::read(destination.path().join("config/acceptance-contract.json")).unwrap(),
        b"project data"
    );
    std::fs::write(&same_named, b"changed project data").unwrap();
    assert_ne!(
        before,
        data_digest(source.path(), &repository(&source), Some(&task)).unwrap()
    );
    assert_eq!(
        std::fs::read(destination.path().join("tasks/set/input.json")).unwrap(),
        b"stable input"
    );
}

#[test]
fn changed_check_data_invalidates_the_digest() {
    let source = tempfile::tempdir().unwrap();
    let input = source.path().join("check-data.json");
    std::fs::write(&input, b"first").unwrap();
    let before = data_digest(source.path(), &repository(&source), None).unwrap();
    std::fs::write(input, b"changed").unwrap();
    assert_ne!(
        before,
        data_digest(source.path(), &repository(&source), None).unwrap()
    );
}

#[test]
fn repository_alias_is_canonicalized_before_project_data_is_digested() {
    let source = tempfile::tempdir().unwrap();
    let repository = source.path().join("repository");
    let nested = source.path().join("nested");
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(repository.join("tracked-input"), b"repository content").unwrap();
    let alias = nested.join("..").join("repository");

    assert_eq!(
        data_digest(source.path(), &repository, None).unwrap(),
        data_digest(source.path(), &alias, None).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn relative_and_symlinked_task_roots_exclude_the_same_contract() {
    let source = tempfile::tempdir().unwrap();
    let task = source.path().join("tasks/set");
    std::fs::create_dir_all(&task).unwrap();
    let contract = task.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    std::fs::write(&contract, b"first contract").unwrap();
    let alias = source.path().join("task-alias");
    std::os::unix::fs::symlink(&task, &alias).unwrap();

    let relative_root = Path::new("tasks/set");
    let relative_before =
        data_digest(source.path(), &repository(&source), Some(relative_root)).unwrap();
    let alias_before = data_digest(source.path(), &repository(&source), Some(&alias)).unwrap();
    std::fs::write(&contract, b"changed contract").unwrap();
    assert_eq!(
        relative_before,
        data_digest(source.path(), &repository(&source), Some(relative_root)).unwrap()
    );
    assert_eq!(
        alias_before,
        data_digest(source.path(), &repository(&source), Some(&alias)).unwrap()
    );
}
