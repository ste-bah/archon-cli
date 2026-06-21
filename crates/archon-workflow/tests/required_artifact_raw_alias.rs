use archon_workflow::{WorkflowExecutor, WorkflowPolicy, WorkflowSpec, WorkflowStore};

#[test]
fn quality_gate_does_not_apply_domain_specific_raw_csv_aliases() {
    let temp = tempfile::tempdir().unwrap();
    let raw_path = temp
        .path()
        .join(".archon/trading-lab/data/datasets/openbb-SPY-1D-raw/v1/raw/response.json");
    std::fs::create_dir_all(raw_path.parent().unwrap()).unwrap();
    std::fs::write(&raw_path, "{}").unwrap();

    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(raw_csv_alias_artifact_spec()).unwrap();

    let report = executor.execute(run).unwrap();
    assert_eq!(report.failed, 1);
}

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

fn raw_csv_alias_artifact_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: exact-raw-csv-artifact-gate
task: Verify exact required artifact paths.
stages:
  - id: final-quality
    kind: quality_gate
    required_artifacts:
      - .archon/trading-lab/data/datasets/*/*/raw.csv
"#,
    )
    .unwrap()
}
