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
    assert!(acceptance_input_excluded(
        &task_contract,
        &excludes,
        Some(Path::new("tasks/set"))
    ));
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
        assert!(acceptance_input_excluded(
            &path,
            &excludes,
            Some(Path::new("tasks/set"))
        ));
        assert!(excluded(&path, &excludes));
    }
    assert!(!excluded(Path::new("tasks/set/input.json"), &excludes));
    assert!(excluded(Path::new("target/debug/build/output"), &excludes));
    assert!(excluded(Path::new("target/debug/build/output"), &[]));
    assert!(acceptance_input_excluded(
        Path::new("tasks/set/TASK-EXAMPLE-001.md"),
        &excludes,
        Some(Path::new("tasks/set"))
    ));
    assert!(acceptance_input_excluded(
        &Path::new("tasks/set").join(crate::task_set_contract::TASK_SKELETON_FILE),
        &excludes,
        Some(Path::new("tasks/set"))
    ));
    assert!(acceptance_input_excluded(
        &Path::new("tasks/set").join(crate::task_set_contract::TASK_SKELETON_LOCK_FILE),
        &excludes,
        Some(Path::new("tasks/set"))
    ));
    assert!(acceptance_input_excluded(
        Path::new("tasks/set/.decompose.log"),
        &excludes,
        Some(Path::new("tasks/set"))
    ));
    assert!(!acceptance_input_excluded(
        Path::new("src/TASK-EXAMPLE-001.md"),
        &excludes,
        Some(Path::new("tasks/set"))
    ));
}

#[cfg(test)]
#[test]
fn live_tripwire_tracks_project_relative_task_inputs_and_skips_freeze_files() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let task_root = project.join("tasks/set");
    let repository = temp.path().join("repository");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    for (path, bytes) in [
        ("config.json", b"config one".as_slice()),
        ("target/build/output", b"target one".as_slice()),
        (".git/config", b"nested git one".as_slice()),
        (
            crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE,
            b"contract one".as_slice(),
        ),
        (
            crate::task_set_contract::ACCEPTANCE_LOCK_FILE,
            b"lock one".as_slice(),
        ),
    ] {
        let path = task_root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repository)
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
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "user.name", "test"]);
    git(&["commit", "-q", "--allow-empty", "-m", "fixture"]);
    let commit = git(&["rev-parse", "HEAD"]);
    let policy = ScratchPolicy {
        repository,
        project: project.clone(),
        task_root: task_root.clone(),
        scratch_parent: temp.path().join("scratch"),
        project_inputs: vec![PathBuf::from("tasks/set")],
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

    let before = live(&policy, &commit).unwrap();
    let task_snapshot = &before[&task_root.display().to_string()];
    for path in [
        "tasks/set/config.json",
        "tasks/set/target/build/output",
        "tasks/set/.git/config",
    ] {
        assert!(task_snapshot.contains_key(path), "{path}");
    }
    for name in [
        crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE,
        crate::task_set_contract::ACCEPTANCE_LOCK_FILE,
    ] {
        assert!(!task_snapshot.contains_key(&format!("tasks/set/{name}")));
        assert!(!before[&project.display().to_string()].contains_key(&format!("tasks/set/{name}")));
    }
    std::fs::write(task_root.join("config.json"), b"config changed").unwrap();
    let after = live(&policy, &commit).unwrap();
    assert_ne!(
        task_snapshot,
        &after[&task_root.display().to_string()],
        "project-relative task inputs are guarded during observation"
    );
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
        let mut stack = vec![relative.to_path_buf()];
        while let Some(path) = stack.pop() {
            if !path.as_os_str().is_empty()
                && acceptance_input_excluded(&path, excludes, Some(Path::new("tasks/set")))
            {
                continue;
            }
            let source = root.join(&path);
            let metadata = match source.symlink_metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => panic!("{}: {error}", source.display()),
            };
            if metadata.is_dir() {
                for item in std::fs::read_dir(&source).unwrap() {
                    stack.push(path.join(item.unwrap().file_name()));
                }
            } else if metadata.is_file() {
                map.insert(path.display().to_string(), file_digest(&source).unwrap());
            }
        }
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
        copy_project(
            &project,
            input,
            &destination,
            (&excludes, None),
            Path::new("tasks/set"),
            &mut remaining,
        )
        .unwrap();
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
    audit_path(temp.path(), relative, (&[], None), None, &mut map).unwrap();
    assert_eq!(
        map.get("config/acceptance-contract.json"),
        Some(&file_digest(&file).unwrap())
    );
}

#[cfg(test)]
#[test]
fn copied_file_sets_match_digest_inputs_for_scratch_and_hermetic_scopes() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let task_relative = Path::new("tasks/set");
    let task_root = project.join(task_relative);
    let fixtures = [
        (".archon/project/data/registry.json", b"registry".as_slice()),
        (".archon/logs/run.jsonl", b"harness".as_slice()),
        (".archon/task-set-pins/set.json", b"pin".as_slice()),
        ("target/debug/build/output", b"build".as_slice()),
        ("tasks/set/acceptance-contract.json", b"contract".as_slice()),
        ("tasks/set/acceptance-contract.lock", b"lock".as_slice()),
        ("tasks/set/task-skeleton.json", b"skeleton".as_slice()),
        ("tasks/set/task-skeleton.lock", b"skeleton lock".as_slice()),
        ("tasks/set/.decompose.log", b"decomposition log".as_slice()),
        ("tasks/set/TASK-NAMED-001.md", b"named".as_slice()),
        ("tasks/set/TASK-UNNAMED-002.md", b"unnamed".as_slice()),
        ("tasks/set/user-notes.txt", b"notes".as_slice()),
        ("tasks/set/excluded/private.txt", b"excluded".as_slice()),
        ("tasks/set/config.json", b"task config".as_slice()),
        ("tasks/set/target/build/output", b"task target".as_slice()),
        ("tasks/set/.git/config", b"task git".as_slice()),
    ];
    for (relative, bytes) in fixtures {
        let path = project.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    std::fs::create_dir_all(&task_root).unwrap();
    let policy = ScratchPolicy {
        repository: temp.path().join("repository"),
        project: project.clone(),
        task_root: task_root.clone(),
        scratch_parent: temp.path().join("scratch"),
        project_inputs: vec![PathBuf::from(".archon"), task_relative.to_path_buf()],
        project_input_excludes: vec![PathBuf::from("tasks/set/excluded")],
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
    let digest_inputs = |scope: &Path| {
        let mut map = BTreeMap::new();
        audit_path(
            &project,
            scope,
            (&excludes, None),
            Some(task_relative),
            &mut map,
        )
        .unwrap();
        map.into_iter()
            .filter_map(|(path, digest)| {
                (digest != "directory" && digest != "missing").then_some(PathBuf::from(path))
            })
            .collect::<BTreeSet<_>>()
    };
    let copied_files = |destination: &Path| {
        let mut files = BTreeSet::new();
        let mut stack = vec![PathBuf::new()];
        while let Some(relative) = stack.pop() {
            let path = destination.join(&relative);
            for entry in std::fs::read_dir(&path).unwrap() {
                let entry = entry.unwrap();
                let child = relative.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    stack.push(child);
                } else {
                    files.insert(child);
                }
            }
        }
        files
    };

    let scratch = temp.path().join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut remaining = u64::MAX;
    copy_project(
        &project,
        Path::new(".archon"),
        &scratch,
        (&excludes, None),
        task_relative,
        &mut remaining,
    )
    .unwrap();
    super::super::io::copy_task_root(
        &task_root,
        &scratch.join(task_relative),
        task_relative,
        task_relative,
        (&excludes, None),
        &mut remaining,
    )
    .unwrap();
    let mut scratch_digest_inputs = digest_inputs(Path::new(".archon"));
    scratch_digest_inputs.extend(digest_inputs(task_relative));
    assert_eq!(copied_files(&scratch), scratch_digest_inputs);
    for path in [
        "tasks/set/acceptance-contract.json",
        "tasks/set/acceptance-contract.lock",
        "tasks/set/task-skeleton.json",
        "tasks/set/task-skeleton.lock",
        "tasks/set/.decompose.log",
        "tasks/set/TASK-NAMED-001.md",
        "tasks/set/TASK-UNNAMED-002.md",
    ] {
        assert!(!copied_files(&scratch).contains(Path::new(path)), "{path}");
        assert!(
            !digest_inputs(task_relative).contains(Path::new(path)),
            "{path}"
        );
    }
    for path in [
        "tasks/set/config.json",
        "tasks/set/target/build/output",
        "tasks/set/.git/config",
    ] {
        assert!(copied_files(&scratch).contains(Path::new(path)), "{path}");
        assert!(
            digest_inputs(task_relative).contains(Path::new(path)),
            "{path}"
        );
    }
}
use std::collections::BTreeSet;
