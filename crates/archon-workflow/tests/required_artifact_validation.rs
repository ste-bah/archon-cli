use archon_workflow::{
    RunStatus, StageStatus, WorkflowExecutor, WorkflowPolicy, WorkflowSpec, WorkflowStore,
};

#[test]
fn quality_gate_fails_when_required_project_artifact_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_artifact_spec();
    let run = executor.start(spec).unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!((report.blocked, report.failed), (0, 1));

    let finished = store.load_state(&run.id).unwrap();
    let gate = finished.stages.get("final-quality").unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
    assert_eq!(gate.status, StageStatus::Failed);
    let error = gate.error.as_deref().unwrap_or_default();
    assert!(error.contains("quality gate missing required artifact"));
    assert!(error.contains(".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json"));

    let quality = std::fs::read_to_string(
        store
            .run_dir(&run.id)
            .join("quality")
            .join("final-quality.json"),
    )
    .unwrap();
    assert!(quality.contains("\"status\": \"failed\""));
    assert!(quality.contains("strategy-spec.json"));
}

#[test]
fn quality_gate_accepts_existing_required_project_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let artifact = temp
        .path()
        .join(".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json");
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(
        &artifact,
        r#"{"artifact":"strategy-spec","status":"ready"}"#,
    )
    .unwrap();

    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_artifact_spec();
    let run = executor.start(spec).unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 0);

    let finished = store.load_state(&run.id).unwrap();
    let gate = finished.stages.get("final-quality").unwrap();
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(gate.status, StageStatus::Accepted);
}

#[test]
fn quality_gate_rejects_placeholder_required_project_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let artifact = temp
        .path()
        .join(".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json");
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(&artifact, "{}").unwrap();

    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_artifact_spec();
    let run = executor.start(spec).unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 1);

    let finished = store.load_state(&run.id).unwrap();
    let error = finished
        .stages
        .get("final-quality")
        .and_then(|stage| stage.error.as_deref())
        .unwrap_or_default();
    assert!(error.contains("invalid required artifact"));
}

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

fn required_artifact_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: required-artifact-gate
task: Verify required project artifact.
stages:
  - id: final-quality
    kind: quality_gate
    required_artifacts:
      - .archon/trading-lab/strategies/AHDM-v1/strategy-spec.json
"#,
    )
    .unwrap()
}
