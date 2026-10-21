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
        project: site.path().join("project"),
        siblings: Vec::new(),
        check_siblings: std::collections::BTreeMap::from([(
            "AC-TEST-001".into(),
            vec![super::super::hermetic::SiblingRevision {
                path: "repository-alias".into(),
                commit: None,
                target: Some(copied_repository.join("tests")),
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
