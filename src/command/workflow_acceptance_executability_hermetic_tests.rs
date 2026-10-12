use super::*;
use std::io::Write;

#[test]
fn repository_path_comparison_normalizes_verbatim_and_plain_spellings() {
    let verbatim = Path::new(r"\\?\C:\work\repository");
    let plain = Path::new(r"C:\work\repository");
    assert!(same_path(verbatim, plain));
}

#[test]
fn task_root_relative_normalizes_verbatim_and_plain_project_spellings() {
    let verbatim = archon_shell::paths::plain(PathBuf::from(r"\\?\C:\work\project"));
    let plain = PathBuf::from(r"C:\work\project");
    assert_eq!(verbatim, plain);

    let directory = tempfile::tempdir().unwrap();
    let project = archon_shell::paths::canonicalize(directory.path()).unwrap();
    let task = project.join("tasks/set");
    std::fs::create_dir_all(&task).unwrap();
    assert_eq!(
        task_root_relative_paths(&project, &task).unwrap(),
        PathBuf::from("tasks/set")
    );
}

#[cfg(windows)]
#[test]
fn canonicalized_repository_path_compares_with_its_plain_spelling() {
    let directory = tempfile::tempdir().unwrap();
    let verbatim = std::fs::canonicalize(directory.path()).unwrap();
    let plain = archon_shell::paths::canonicalize(directory.path()).unwrap();
    assert!(same_path(&verbatim, &plain));
}

#[cfg(windows)]
#[test]
fn task_root_digest_and_hermetic_copy_accept_a_windows_task_root() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path();
    let repository = repository(&source);
    let commit = committed_repository(&repository);
    let task_root = project.join("tasks/set");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::write(task_root.join("input.txt"), b"task input").unwrap();

    assert!(data_digest(project, &repository, Some(&task_root)).is_ok());
    let copy = HermeticCopy::at(
        destination.path(),
        &repository,
        project,
        Some(&task_root),
        &commit,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(copy.project.join("tasks/set/input.txt")).unwrap(),
        b"task input"
    );
    copy.remove().unwrap();
}

fn repository(source: &tempfile::TempDir) -> PathBuf {
    let path = source.path().join("repository");
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn committed_repository(path: &Path) -> String {
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.invalid"]);
    run(&["config", "user.name", "test"]);
    run(&["commit", "-q", "--allow-empty", "-m", "fixture"]);
    run(&["rev-parse", "HEAD"])
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
fn hermetic_copy_file_set_matches_its_digest_inputs() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path().canonicalize().unwrap();
    let repository = repository(&source);
    let commit = committed_repository(&repository);
    let task_root = project.join("tasks/set");
    let files = [
        (".archon/project/data/registry.json", b"registry".as_slice()),
        (".archon/logs/run.jsonl", b"harness".as_slice()),
        (".archon/task-set-pins/set.json", b"pin".as_slice()),
        ("target/debug/build/output", b"build".as_slice()),
        ("tasks/set/acceptance-contract.json", b"contract".as_slice()),
        ("tasks/set/acceptance-contract.lock", b"lock".as_slice()),
        ("tasks/set/task-skeleton.json", b"skeleton".as_slice()),
        ("tasks/set/task-skeleton.lock", b"skeleton lock".as_slice()),
        ("tasks/set/TASK-NAMED-001.md", b"named".as_slice()),
        ("tasks/set/TASK-UNNAMED-002.md", b"unnamed".as_slice()),
        ("tasks/set/user-notes.txt", b"notes".as_slice()),
        ("tasks/set/config.json", b"task config".as_slice()),
    ];
    for (relative, bytes) in files {
        let path = project.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    std::fs::create_dir_all(&task_root).unwrap();
    let task_root = task_root.canonicalize().unwrap();

    let before = data_digest(&project, &repository, Some(&task_root)).unwrap();
    let copy = HermeticCopy::at(
        destination.path(),
        &repository,
        &project,
        Some(&task_root),
        &commit,
    )
    .unwrap();
    let mut copied_files = std::collections::BTreeSet::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(relative) = stack.pop() {
        for entry in std::fs::read_dir(copy.project.join(&relative)).unwrap() {
            let entry = entry.unwrap();
            let child = relative.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                stack.push(child);
            } else {
                copied_files.insert(child);
            }
        }
    }
    for path in [
        "tasks/set/acceptance-contract.json",
        "tasks/set/acceptance-contract.lock",
        "tasks/set/task-skeleton.json",
        "tasks/set/task-skeleton.lock",
        "tasks/set/TASK-NAMED-001.md",
        "tasks/set/TASK-UNNAMED-002.md",
    ] {
        assert!(!copied_files.contains(Path::new(path)), "{path}");
    }
    assert!(copied_files.contains(Path::new("tasks/set/config.json")));
    assert_eq!(
        before,
        data_digest(&project, &repository, Some(&task_root)).unwrap(),
        "excluded task-root artifacts do not enter the production digest"
    );
    for path in [
        "acceptance-contract.json",
        "acceptance-contract.lock",
        "task-skeleton.json",
        "task-skeleton.lock",
        "TASK-NAMED-001.md",
        "TASK-UNNAMED-002.md",
    ] {
        std::fs::write(task_root.join(path), b"changed excluded data").unwrap();
    }
    assert_eq!(
        before,
        data_digest(&project, &repository, Some(&task_root)).unwrap()
    );
    copy.remove().unwrap();
}

#[test]
fn copy_data_rejects_a_task_root_outside_the_project() {
    let source = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let root = source.path().canonicalize().unwrap();
    let mut remaining = u64::MAX;
    let error = copy_data(
        &root,
        Path::new(""),
        destination.path(),
        &repository(&source),
        Some(outside.path()),
        &mut remaining,
    )
    .unwrap_err();
    assert!(error.0.contains("task root"), "{}", error.0);
    assert!(error.0.contains("outside project"), "{}", error.0);
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
