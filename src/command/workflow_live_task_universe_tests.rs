use super::*;

#[test]
fn universe_comes_from_task_files_not_reducer_items() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: []\n",
    )
    .expect("task 1");
    fs::write(
        temp.path().join("TASK-TDL-010-dependent.md"),
        "# Dependent\n\ntask_id: TASK-TDL-010\ndepends_on: ['TASK-TDL-001']\n",
    )
    .expect("task 10");

    let universe = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect("extract")
    .expect("universe");

    assert_eq!(
        universe.canonical_ids(),
        vec!["TASK-TDL-001".to_string(), "TASK-TDL-010".to_string()]
    );
    assert_eq!(
        universe.tasks[1].dependency_ids,
        vec!["TASK-TDL-001".to_string()]
    );
}

#[test]
fn prd_task_references_must_have_matching_task_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let prd = temp.path().join("PRD.md");
    fs::write(&prd, "Acceptance references TASK-TDL-140.\n").expect("prd");
    let tasks = temp.path().join("tasks");
    fs::create_dir_all(&tasks).expect("tasks");
    fs::write(
        tasks.join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: []\n",
    )
    .expect("task");

    let err = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {} and tasks at {}",
        prd.display(),
        tasks.display()
    ))
    .expect_err("unbacked PRD task reference must fail");

    assert!(err.to_string().contains("references TASK-TDL-140"));
}

#[test]
fn missing_authoritative_task_evidence_fails_for_decomposed_prd() {
    let err = extract_task_universe_for_generated_run(
        "Implement the decomposed PRD at /no/such/tasks/PRD-MISSING",
    )
    .expect_err("missing local evidence must fail");

    assert!(
        err.to_string()
            .contains("requires local TASK-*.md evidence")
    );
}

#[test]
fn invalid_task_id_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-1\ndepends_on: []\n",
    )
    .expect("task");

    let err = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect_err("invalid task id must fail");

    assert!(err.to_string().contains("invalid task_id"));
}

#[test]
fn dependency_cycles_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: [TASK-TDL-010]\n",
    )
    .expect("task 1");
    fs::write(
        temp.path().join("TASK-TDL-010-dependent.md"),
        "# Dependent\n\ntask_id: TASK-TDL-010\ndepends_on: [TASK-TDL-001]\n",
    )
    .expect("task 10");

    let err = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect_err("cycle must fail");

    assert!(err.to_string().contains("dependency cycle"));
}

#[test]
fn neutral_task_and_project_capabilities_are_loaded_from_declarations() {
    let project = tempfile::tempdir().expect("project");
    let archon = project.path().join(".archon");
    let tasks = project.path().join("tasks/PRD-DEMO");
    fs::create_dir_all(&archon).expect("archon dir");
    fs::create_dir_all(&tasks).expect("tasks dir");
    fs::write(
        archon.join("project.json"),
        serde_json::json!({
            "required_env_keys": ["PROJECT_TOKEN"],
            "required_tools": ["project_probe"]
        })
        .to_string(),
    )
    .expect("project manifest");
    fs::write(
        tasks.join("TASK-DEMO-017-deliverable.md"),
        r#"# Neutral deliverable

```yaml
task_id: TASK-DEMO-017
depends_on: []
required_env_keys: [TASK_TOKEN]
required_tools: [fetch_demo]
deliverable_contracts:
  - kind: required_universe_registry
    artifact_path: .archon/demo/coverage.json
    registry_path: .archon/demo/registry.json
    required_universe: true
```
"#,
    )
    .expect("task");

    let universe = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD task files at {}",
        tasks.display()
    ))
    .expect("extract")
    .expect("universe");
    let task = &universe.tasks[0];

    assert_eq!(task.canonical_task_id, "TASK-DEMO-017");
    assert_eq!(
        task.required_env_keys,
        vec!["PROJECT_TOKEN".to_string(), "TASK_TOKEN".to_string()]
    );
    assert_eq!(
        task.required_tools,
        vec!["fetch_demo".to_string(), "project_probe".to_string()]
    );
    assert_eq!(task.deliverable_contracts.len(), 1);
    assert!(task.deliverable_contracts[0].required_universe);
}

#[test]
fn runtime_workflow_code_contains_no_fixture_task_ids() {
    let command_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/command");
    for entry in fs::read_dir(&command_dir).expect("read command sources") {
        let path = entry.expect("source entry").path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("workflow_live") || !name.ends_with(".rs") || name.contains("_tests") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("read runtime source");
        let runtime_only = source.split("\n#[cfg(test)]").next().unwrap_or(&source);
        assert!(
            !runtime_only.to_ascii_lowercase().contains("task-tdl"),
            "fixture task id leaked into runtime source {}",
            path.display()
        );
    }
}

#[test]
fn malformed_deliverable_contract_fails_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-DEMO-017-invalid.md"),
        r#"```yaml
task_id: TASK-DEMO-017
depends_on: []
deliverable_contracts:
  - kind: required_universe_registry
    artifact_path: 42
```"#,
    )
    .expect("task");

    let error = extract_task_universe_for_generated_run(&format!(
        "Implement decomposed PRD task files at {}",
        temp.path().display()
    ))
    .expect_err("invalid capability contract must fail");

    assert!(error.to_string().contains("invalid type"));
}
