use super::*;

#[cfg(test)]
#[test]
fn freeze_outputs_are_excluded_from_scratch_project_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let task_root = project.join("tasks/set");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::write(
        task_root.join(crate::task_set_contract::TASK_SKELETON_FILE),
        br#"{"schema_version":1,"acceptance_digest":"d","tasks":[{"task_id":"TASK-EXAMPLE-001","file_name":"TASK-EXAMPLE-001.md"}]}"#,
    )
    .unwrap();
    let policy = ScratchPolicy {
        repository: temp.path().join("repository"),
        project: project.clone(),
        task_root,
        scratch_parent: temp.path().join("scratch"),
        project_inputs: Vec::new(),
        project_input_excludes: Vec::new(),
        combined: false,
        toolchain_path: String::new(),
        environment: BTreeMap::new(),
        environment_allowlist: Vec::new(),
        cargo_seed: None,
        timeout_secs: 1,
        output_bytes: 1,
        scratch_bytes: 1,
        build_cache: None,
    };
    let excludes = project_excludes(&policy).unwrap();
    let task_contract =
        Path::new("tasks/set").join(crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    assert!(excluded(&task_contract, &excludes));
    assert!(!excluded(
        Path::new(crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        &excludes
    ));
    for path in [
        Path::new("tasks/set").join(crate::task_set_contract::ACCEPTANCE_LOCK_FILE),
        Path::new(".archon")
            .join(crate::task_set_lineage::PIN_STORE_NAMESPACE)
            .join("set.json"),
    ] {
        assert!(excluded(&path, &excludes), "{}", path.display());
    }
    assert!(!excluded(Path::new("tasks/set/input.json"), &excludes));
    assert!(excluded(Path::new("target/debug/build/output"), &excludes));
    assert!(excluded(
        Path::new("tasks/set/TASK-EXAMPLE-001.md"),
        &excludes
    ));
    assert!(excluded(
        &Path::new("tasks/set").join(crate::task_set_contract::TASK_SKELETON_FILE),
        &excludes
    ));
    assert!(excluded(
        &Path::new("tasks/set").join(crate::task_set_contract::TASK_SKELETON_LOCK_FILE),
        &excludes
    ));
    assert!(excluded(Path::new("tasks/set/.decompose.log"), &excludes));
    assert!(!excluded(Path::new("src/TASK-EXAMPLE-001.md"), &excludes));
}

#[cfg(test)]
#[test]
fn pipeline_task_root_outputs_do_not_change_input_digest() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let task_root = project.join("tasks/set");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::write(task_root.join("TASK-EXAMPLE-001.md"), b"body one").unwrap();
    std::fs::write(
        task_root.join(crate::task_set_contract::TASK_SKELETON_FILE),
        br#"{"schema_version":1,"acceptance_digest":"one","tasks":[{"task_id":"TASK-EXAMPLE-001","file_name":"TASK-EXAMPLE-001.md"}]}"#,
    )
    .unwrap();
    std::fs::write(
        task_root.join(crate::task_set_contract::TASK_SKELETON_LOCK_FILE),
        b"lock one",
    )
    .unwrap();
    std::fs::write(task_root.join(".decompose.log"), b"log one").unwrap();
    std::fs::write(task_root.join("user-notes.txt"), b"user one").unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/data.txt"), b"project one").unwrap();
    let policy = ScratchPolicy {
        repository: temp.path().join("repository"),
        project: project.clone(),
        task_root: task_root.clone(),
        scratch_parent: temp.path().join("scratch"),
        project_inputs: Vec::new(),
        project_input_excludes: Vec::new(),
        combined: false,
        toolchain_path: String::new(),
        environment: BTreeMap::new(),
        environment_allowlist: Vec::new(),
        cargo_seed: None,
        timeout_secs: 1,
        output_bytes: 1,
        scratch_bytes: 1,
        build_cache: None,
    };
    let excludes = project_excludes(&policy).unwrap();
    let digest = |root: &Path, relative: &Path, excludes: &[PathBuf]| {
        let mut map = BTreeMap::new();
        audit_path(root, relative, excludes, &mut map).unwrap();
        map
    };
    let initial = digest(&project, Path::new(""), &excludes);
    std::fs::write(task_root.join("TASK-EXAMPLE-001.md"), b"body two").unwrap();
    std::fs::write(
        task_root.join(crate::task_set_contract::TASK_SKELETON_FILE),
        br#"{"schema_version":1,"acceptance_digest":"two","tasks":[{"task_id":"TASK-EXAMPLE-001","file_name":"TASK-EXAMPLE-001.md"}]}"#,
    )
    .unwrap();
    std::fs::write(
        task_root.join(crate::task_set_contract::TASK_SKELETON_LOCK_FILE),
        b"lock two",
    )
    .unwrap();
    std::fs::write(task_root.join(".decompose.log"), b"log two").unwrap();
    assert_eq!(initial, digest(&project, Path::new(""), &excludes));

    std::fs::write(project.join("src/data.txt"), b"project two").unwrap();
    assert_ne!(initial, digest(&project, Path::new(""), &excludes));
    let after_project_change = digest(&project, Path::new(""), &excludes);
    std::fs::write(task_root.join("user-notes.txt"), b"user two").unwrap();
    assert_ne!(
        after_project_change,
        digest(&project, Path::new(""), &excludes)
    );
}

#[cfg(test)]
#[test]
fn project_input_copy_keeps_archon_data_and_omits_harness_and_pipeline_outputs() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let task_root = project.join("tasks/set");
    let data = project.join(".archon/project/data/registry.json");
    std::fs::create_dir_all(data.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::create_dir_all(project.join(".archon/logs")).unwrap();
    std::fs::write(&data, b"project data").unwrap();
    std::fs::write(
        project.join(".archon/logs/workflow-gates-shadow.jsonl"),
        b"harness log",
    )
    .unwrap();
    std::fs::write(project.join(".archon/learning-state.db"), b"harness db").unwrap();
    std::fs::write(task_root.join("user-notes.txt"), b"task input").unwrap();
    std::fs::write(task_root.join("TASK-EXAMPLE-001.md"), b"pipeline body").unwrap();
    std::fs::write(
        task_root.join(crate::task_set_contract::TASK_SKELETON_FILE),
        br#"{"schema_version":1,"acceptance_digest":"d","tasks":[{"task_id":"TASK-EXAMPLE-001","file_name":"TASK-EXAMPLE-001.md"}]}"#,
    )
    .unwrap();
    let policy = ScratchPolicy {
        repository: temp.path().join("repository"),
        project: project.clone(),
        task_root: task_root.clone(),
        scratch_parent: temp.path().join("scratch"),
        project_inputs: vec![PathBuf::from(".archon"), PathBuf::from("tasks/set")],
        project_input_excludes: Vec::new(),
        combined: false,
        toolchain_path: String::new(),
        environment: BTreeMap::new(),
        environment_allowlist: Vec::new(),
        cargo_seed: None,
        timeout_secs: 1,
        output_bytes: 1,
        scratch_bytes: 1,
        build_cache: None,
    };
    let excludes = project_excludes(&policy).unwrap();
    let destination = temp.path().join("copy");
    std::fs::create_dir_all(&destination).unwrap();
    let mut remaining = u64::MAX;
    for input in &policy.project_inputs {
        copy_project(&project, input, &destination, &excludes, &mut remaining).unwrap();
    }

    assert_eq!(
        std::fs::read(destination.join(".archon/project/data/registry.json")).unwrap(),
        b"project data"
    );
    assert!(
        !destination
            .join(".archon/logs/workflow-gates-shadow.jsonl")
            .exists()
    );
    assert!(!destination.join(".archon/learning-state.db").exists());
    assert_eq!(
        std::fs::read(destination.join("tasks/set/user-notes.txt")).unwrap(),
        b"task input"
    );
    assert!(!destination.join("tasks/set/TASK-EXAMPLE-001.md").exists());
    assert!(
        !destination
            .join("tasks/set")
            .join(crate::task_set_contract::TASK_SKELETON_FILE)
            .exists()
    );
}

#[cfg(test)]
#[test]
fn same_named_nested_project_data_is_included_and_digested() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("config/acceptance-contract.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"project data").unwrap();
    let relative = Path::new("config/acceptance-contract.json");
    assert!(!excluded(relative, &[]));
    let mut map = BTreeMap::new();
    audit_path(temp.path(), relative, &[], &mut map).unwrap();
    assert_eq!(
        map.get("config/acceptance-contract.json"),
        Some(&file_digest(&file).unwrap())
    );
}
