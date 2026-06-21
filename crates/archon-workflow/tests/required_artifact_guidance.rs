use archon_workflow::{WorkflowExecutor, WorkflowPolicy, WorkflowSpec, WorkflowStore};

#[test]
fn artifact_inventory_includes_generic_materialization_guidance() {
    let project = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(project.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(dataset_artifact_spec()).unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 1);

    let body = std::fs::read_to_string(
        store
            .run_dir(&run.id)
            .join("artifacts/required-artifact-inventory/required-artifact-inventory.json"),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    let guidance = &value["items"][0]["repair_guidance"];
    assert_eq!(
        guidance["repair_mode"].as_str(),
        Some("materialize_or_repair_source_then_explain")
    );
    assert_eq!(guidance["must_attempt_generation"].as_bool(), Some(true));
    let commands = guidance["candidate_commands"].as_array().unwrap();
    assert!(!commands.is_empty());
    assert!(
        commands.iter().any(|command| command["command"]
            .as_str()
            .is_some_and(|text| text.contains("source ./profile"))),
        "candidate commands must load project environment: {commands:?}"
    );
    assert_eq!(
        guidance["command_discovery_required_if_blocked"].as_bool(),
        Some(true)
    );
}

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

fn dataset_artifact_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: dataset-artifact-gate
task: Verify required workflow artifacts.
stages:
  - id: final-quality
    kind: quality_gate
    required_artifacts:
      - .archon/demo/artifacts/*/manifest.json
"#,
    )
    .unwrap()
}
