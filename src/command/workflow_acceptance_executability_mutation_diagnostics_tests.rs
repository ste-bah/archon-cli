use super::*;

fn entry(command: &str, cwd: &str) -> AcceptanceCriterion {
    serde_json::from_value(serde_json::json!({
        "id": "AC-TEST-001",
        "criterion": "the command checks its input",
        "check": {"kind": "command", "command": command, "cwd": cwd},
        "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": "test"}
    }))
    .unwrap()
}

#[test]
fn discovery_keeps_typed_reasons_for_text_writes_sibling_manifest_and_missing_paths() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(project.join("fixtures")).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(sibling.join("input.txt"), "ready\n").unwrap();
    let command = "printf ready > generated.txt && grep -q ready generated.txt && test -f ../sibling/input.txt && test -f Cargo.toml && test -f fixtures/missing.txt";
    let result = input_discovery(&entry(command, "project_root"), &project, &project);
    let reason = |path: &str| {
        result
            .candidates
            .iter()
            .find(|candidate| candidate.path == path)
            .unwrap_or_else(|| panic!("candidate {path} missing from {:?}", result.candidates))
            .exclusion
    };
    assert_eq!(
        reason("generated.txt"),
        Some(ExclusionReason::CheckWritesPathInText)
    );
    assert!(!result.eligible.iter().any(|path| path == "generated.txt"));
    assert!(ExclusionReason::CheckWritesPathInText.label().contains(
        "a command in the check writes this path (seen in the check text), so it is not a pre-existing input"
    ));
    assert_eq!(
        reason("../sibling/input.txt"),
        Some(ExclusionReason::SiblingFromRoot)
    );
    assert_eq!(reason("Cargo.toml"), Some(ExclusionReason::ManifestOrSetup));
    assert_eq!(
        reason("fixtures/missing.txt"),
        Some(ExclusionReason::MissingAtRoot)
    );
}

#[test]
fn repository_root_can_name_an_existing_focused_test_as_a_mutation_input() {
    let project = tempfile::tempdir().unwrap();
    let repository = tempfile::tempdir().unwrap();
    std::fs::create_dir(repository.path().join("tests")).unwrap();
    std::fs::write(repository.path().join("tests/focused.rs"), "#[test]\n").unwrap();
    assert_eq!(
        input_discovery(
            &entry("test -s tests/focused.rs", "repo_root"),
            project.path(),
            repository.path()
        )
        .eligible,
        ["tests/focused.rs"]
    );
}

#[test]
fn repository_root_operands_are_excluded_with_accurate_reason_and_cd_options_are_mechanics() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let repository = parent.path().join("repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(repository.join("tests")).unwrap();
    std::fs::write(repository.join("tests/focused.rs"), "focused").unwrap();
    for command in [
        "test -d ../repo",
        "ls ../repo",
        "cd -P ../repo",
        "cd -L ../repo",
    ] {
        let result = input_discovery(&entry(command, "project_root"), &project, &repository);
        let root = result
            .candidates
            .iter()
            .find(|candidate| candidate.path == "../repo")
            .unwrap_or_else(|| {
                panic!(
                    "repository root candidate missing for {command}: {:?}",
                    result.candidates
                )
            });
        assert!(
            !result.eligible.iter().any(|path| path == "../repo"),
            "root was eligible for {command}: {:?}",
            result.candidates
        );
        if command.starts_with("cd -") {
            assert_eq!(
                root.exclusion,
                Some(ExclusionReason::CheckMechanics),
                "cd option operand was not classified as mechanics: {:?}",
                result.candidates
            );
        } else {
            assert!(
                root.exclusion
                    .is_some_and(|reason| reason.label().contains("repository root itself")),
                "inaccurate root reason for {command}: {:?}",
                root.exclusion
            );
        }
    }
}

#[test]
fn message_only_paths_stay_excluded_as_check_mechanics() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("tests")).unwrap();
    std::fs::write(root.path().join("tests/focused.rs"), "focused\n").unwrap();
    let result = input_discovery(
        &entry("echo 'tests/focused.rs'", "project_root"),
        root.path(),
        root.path(),
    );
    let path = result
        .candidates
        .iter()
        .find(|candidate| candidate.path == "tests/focused.rs")
        .expect("message-only path is still diagnosed");
    assert_eq!(path.exclusion, Some(ExclusionReason::CheckMechanics));
    assert!(
        !result
            .eligible
            .iter()
            .any(|candidate| candidate == "tests/focused.rs")
    );
}

#[test]
fn cd_pipeline_does_not_turn_pipe_into_a_path_operand() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("pipe"), "input\n").unwrap();
    let result = input_discovery(&entry("cd | cmd", "project_root"), root.path(), root.path());
    assert!(
        !result
            .candidates
            .iter()
            .any(|candidate| candidate.path == "pipe"),
        "the pipeline marker became a bogus input: {:?}",
        result.candidates
    );
    assert!(!result.eligible.iter().any(|path| path == "pipe"));
}

#[test]
fn a_sibling_alias_resolving_inside_the_repository_is_movable_from_its_copy() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let repository = parent.path().join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(repository.join("tests")).unwrap();
    std::fs::write(repository.join("tests/focused.rs"), "focused").unwrap();
    let site = tempfile::tempdir().unwrap();
    let copied_repository = site.path().join("repository");
    std::fs::create_dir_all(copied_repository.join("tests")).unwrap();
    std::fs::write(copied_repository.join("tests/focused.rs"), "focused").unwrap();
    std::os::unix::fs::symlink(
        copied_repository.join("tests"),
        site.path().join("repository-alias"),
    )
    .unwrap();
    let layout = super::super::hermetic::CopyLayout {
        root: site.path().to_path_buf(),
        repository: copied_repository.clone(),
        site_repository: Some(copied_repository.clone()),
        project: site.path().join("project"),
        siblings: Vec::new(),
        check_siblings: std::collections::BTreeMap::from([(
            "AC-TEST-001".into(),
            vec![super::super::hermetic::SiblingRevision {
                path: "repository-alias".into(),
                commit: None,
                target: Some(copied_repository.join("tests")),
                site_repository: Some(copied_repository.clone()),
            }],
        )]),
    };
    let result = input_discovery_for_check(
        &entry(
            "grep -q focused ../repository/tests/focused.rs",
            "project_root",
        ),
        &project,
        &repository,
        Some(&layout),
    );
    assert_eq!(result.eligible, ["../repository/tests/focused.rs"]);
    assert_eq!(
        result.candidates[0].move_path.as_deref(),
        Some(copied_repository.join("tests/focused.rs").to_str().unwrap())
    );
    assert_eq!(
        result.candidates[0].move_root.as_deref(),
        Some(copied_repository.to_str().unwrap())
    );
    assert_eq!(
        result.candidates[0].exclusion,
        Some(ExclusionReason::RepositoryCopyAlias)
    );
    assert!(
        ExclusionReason::RepositoryCopyAlias
            .label()
            .contains("resolves inside the copied repository")
    );
}

#[cfg(unix)]
#[test]
fn nested_directory_named_repo_uses_the_recorded_site_repository_root() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let repository = parent.path().join("repository");
    let nested = repository.join("fixtures/repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(nested.join("tests")).unwrap();
    std::fs::write(nested.join("tests/focused.rs"), "focused").unwrap();
    let site = tempfile::tempdir().unwrap();
    let site_repository = site.path().join("repo");
    let site_nested = site_repository.join("fixtures/repo");
    std::fs::create_dir_all(site_nested.join("tests")).unwrap();
    std::fs::write(site_nested.join("tests/focused.rs"), "focused").unwrap();
    std::os::unix::fs::symlink(&nested, project.parent().unwrap().join("alias")).unwrap();
    let layout = super::super::hermetic::CopyLayout {
        root: site.path().to_path_buf(),
        repository: repository.clone(),
        site_repository: Some(site_repository.clone()),
        project: site.path().join("project"),
        siblings: Vec::new(),
        check_siblings: std::collections::BTreeMap::from([(
            "AC-TEST-001".into(),
            vec![super::super::hermetic::SiblingRevision {
                path: PathBuf::from("../alias"),
                commit: None,
                target: Some(site_nested.clone()),
                site_repository: Some(site_repository.clone()),
            }],
        )]),
    };
    let result = input_discovery_for_check(
        &entry("grep -q focused ../alias/tests/focused.rs", "project_root"),
        &project,
        &repository,
        Some(&layout),
    );
    let candidate = result
        .candidates
        .iter()
        .find(|candidate| candidate.move_path.is_some())
        .unwrap();
    assert_eq!(
        candidate.move_path.as_deref(),
        Some(site_nested.join("tests/focused.rs").to_str().unwrap())
    );
    assert_eq!(
        candidate.move_root.as_deref(),
        Some(site_repository.to_str().unwrap())
    );
}

#[cfg(unix)]
#[test]
fn absolute_project_symlink_to_live_repository_is_outside_the_copy() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let repository = parent.path().join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(repository.join("tests")).unwrap();
    std::fs::write(repository.join("tests/focused.rs"), "focused").unwrap();
    std::os::unix::fs::symlink(
        repository.join("tests/focused.rs"),
        project.join("live-test.rs"),
    )
    .unwrap();
    let site = tempfile::tempdir().unwrap();
    let copied_repository = site.path().join("repo");
    std::fs::create_dir_all(copied_repository.join("tests")).unwrap();
    std::fs::write(copied_repository.join("tests/focused.rs"), "focused").unwrap();
    let layout = super::super::hermetic::CopyLayout {
        root: site.path().to_path_buf(),
        repository: repository.clone(),
        site_repository: Some(copied_repository),
        project: site.path().join("project"),
        siblings: Vec::new(),
        check_siblings: std::collections::BTreeMap::new(),
    };
    let result = input_discovery_for_check(
        &entry("grep -q focused live-test.rs", "project_root"),
        &project,
        &repository,
        Some(&layout),
    );
    let candidate = result
        .candidates
        .iter()
        .find(|candidate| {
            candidate.path == "../repository/tests/focused.rs" || candidate.path == "live-test.rs"
        })
        .unwrap();
    assert_eq!(
        candidate.exclusion,
        Some(ExclusionReason::OutsideCopiedTree),
        "absolute symlink must keep its live target semantics: {:?}",
        result.candidates
    );
}
