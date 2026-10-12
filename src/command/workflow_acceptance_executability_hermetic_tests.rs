use super::*;
use std::io::Write;

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
fn harness_run_state_is_excluded_but_archon_project_data_is_copied_and_digested() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let data = source.path().join(".archon/project/data/registry.json");
    std::fs::create_dir_all(data.parent().unwrap()).unwrap();
    std::fs::write(&data, b"registry v1").unwrap();
    let paths = [
        ".archon/logs/workflow-gates-shadow.jsonl",
        ".archon/lint-cache/fidelity/verdict.json",
        ".archon/cognitive/cognitive-decisions.jsonl",
        ".archon/spill/session/call.txt",
        ".archon/topology/graph-a/trace.jsonl",
        ".archon/topology.db",
        ".archon/topology.db.archon-cozo-write.lock.progress",
        ".archon/freeze-cache/probe-results/result.json",
        ".archon/workflows/run/state.json",
        ".archon/task-set-pins/set.json",
        ".archon/learning-state.db",
        ".archon/learning-state.db-wal",
        ".archon/learning-state.db.archon-cozo-write.lock.progress",
        ".archon/leann.db.archon-cozo-write.lock",
    ];
    for path in paths {
        let file = source.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"harness output").unwrap();
    }
    let before = data_digest(source.path(), &repository(&source), None).unwrap();
    let mut remaining = u64::MAX;
    copy_data(
        &source.path().canonicalize().unwrap(),
        Path::new(""),
        destination.path(),
        &repository(&source),
        None,
        &mut remaining,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(
            destination
                .path()
                .join(".archon/project/data/registry.json")
        )
        .unwrap(),
        b"registry v1"
    );
    for path in paths {
        assert!(
            !destination.path().join(path).exists(),
            "scratch copy included harness state {}",
            path
        );
    }
    let log = source.path().join(paths[0]);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"another freeze round\n")
        .unwrap();
    for path in &paths[1..] {
        std::fs::write(source.path().join(path), b"changed harness output").unwrap();
    }
    assert_eq!(
        before,
        data_digest(source.path(), &repository(&source), None).unwrap()
    );
    std::fs::write(&data, b"registry v2").unwrap();
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
