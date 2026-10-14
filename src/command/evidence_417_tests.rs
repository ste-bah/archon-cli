use super::*;

fn roots() -> (tempfile::TempDir, tempfile::TempDir, PathBuf, PathBuf) {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let repository = source.path().join("repository");
    std::fs::create_dir_all(&repository).unwrap();
    let project = source.path().canonicalize().unwrap();
    (source, destination, project, repository)
}

fn digest(project: &Path, repository: &Path, task: Option<&Path>) -> String {
    data_digest(project, repository, task).unwrap()
}

fn evidence(case: &str, before: impl std::fmt::Display, after: impl std::fmt::Display) {
    println!("EVIDENCE #417 {case} BEFORE: {before} AFTER: {after}");
}

// Expected before running: changing freeze-owned files leaves the digest
// unchanged, while a normal project file changes it.
#[test]
fn evidence_417_published_outputs_and_harness_state_do_not_change_digest() {
    let (source, _destination, project, repository) = roots();
    let task = source.path().join("tasks/set");
    std::fs::create_dir_all(&task).unwrap();
    let freeze_files = [
        task.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        task.join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE),
        project.join(".archon/task-set-pins/set.json"),
        project.join(".archon/logs/workflow-gates-shadow.jsonl"),
        project.join(".archon/learning-state.db"),
    ];
    for file in &freeze_files {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"first generated contents").unwrap();
    }
    let project_data = project.join(".archon/project/decisions.json");
    std::fs::create_dir_all(project_data.parent().unwrap()).unwrap();
    std::fs::write(&project_data, b"project-owned data").unwrap();
    let task = task.canonicalize().unwrap();
    let before = digest(&project, &repository, Some(&task));
    for (index, file) in freeze_files.iter().enumerate() {
        std::fs::write(file, format!("generated revision {index}")).unwrap();
    }
    let after_generated = digest(&project, &repository, Some(&task));
    assert_eq!(before, after_generated);
    let source_read = std::fs::read(&project_data).unwrap();
    assert_eq!(source_read, b"project-owned data");
    std::fs::write(&project_data, b"changed project-owned data").unwrap();
    let after_project_edit = digest(&project, &repository, Some(&task));
    assert_ne!(after_generated, after_project_edit);
    evidence(
        "generated-state-excluded-project-data-included",
        format!(
            "digest={before}; generated files present; project bytes={:?}",
            source_read
        ),
        format!(
            "after generated writes={after_generated}; after project edit={after_project_edit}"
        ),
    );
}

// Expected before running: real project edits invalidate the digest and are
// present in the copied probe input tree.
#[test]
fn evidence_417_changed_project_input_invalidates_digest_and_copy() {
    let (source, destination, project, repository) = roots();
    let input = project.join("config/check-input.json");
    std::fs::create_dir_all(input.parent().unwrap()).unwrap();
    std::fs::write(&input, br#"{"version":1}"#).unwrap();
    let before = digest(&project, &repository, None);
    std::fs::write(&input, br#"{"version":2}"#).unwrap();
    let after = digest(&project, &repository, None);
    assert_ne!(before, after);
    let mut remaining = u64::MAX;
    copy_data(
        &project,
        Path::new(""),
        destination.path(),
        &repository,
        None,
        &mut remaining,
    )
    .unwrap();
    let copied = std::fs::read(destination.path().join("config/check-input.json")).unwrap();
    assert_eq!(copied, br#"{"version":2}"#);
    let separately_read = std::fs::read(&input).unwrap();
    assert_eq!(copied, separately_read);
    evidence(
        "changed-check-data-invalidates-and-copies",
        format!("source={:?}; digest={before}", br#"{"version":1}"#),
        format!(
            "source={:?}; copied={:?}; digest={after}",
            separately_read, copied
        ),
    );
    drop(source);
}

// Expected before running: a probe copy omits the published contract, so an
// actual command checking its path reports it absent.
#[test]
fn evidence_417_check_cannot_read_published_contract_from_probe_copy() {
    let (source, destination, project, repository) = roots();
    let task = project.join("tasks/set");
    std::fs::create_dir_all(&task).unwrap();
    let contract = task.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    std::fs::write(&contract, b"published contract bytes").unwrap();
    let task = task.canonicalize().unwrap();
    let mut remaining = u64::MAX;
    copy_data(
        &project,
        Path::new(""),
        destination.path(),
        &repository,
        Some(&task),
        &mut remaining,
    )
    .unwrap();
    let copied_contract = destination
        .path()
        .join("tasks/set")
        .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let command = std::process::Command::new("/bin/sh")
        .args(["-c", "test -e tasks/set/acceptance-contract.json"])
        .current_dir(destination.path())
        .status()
        .unwrap();
    let source_bytes = std::fs::read(&contract).unwrap();
    let copy_exists = copied_contract.exists();
    assert!(!command.success());
    assert!(!copy_exists);
    assert_eq!(source_bytes, b"published contract bytes");
    evidence(
        "probe-command-contract-visibility",
        format!(
            "source bytes={:?}; copied exists={copy_exists}",
            source_bytes
        ),
        format!(
            "command exit={:?}; copied exists={}",
            command.code(),
            copied_contract.exists()
        ),
    );
    drop(source);
}
