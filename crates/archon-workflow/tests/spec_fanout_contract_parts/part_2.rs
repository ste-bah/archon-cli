#[test]
fn generated_targetless_implementation_without_inventory_request_fails_validation() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-targetless-implementation
task: Implement the decomposed PRD.
stages:
  - id: implement_t001
    kind: implementation
    task: Implement TASK-TDL-001.
    provider_tier: coder
"#;
    let err = WorkflowSpec::from_generated_yaml(yaml, "Fallback task")
        .expect_err("targetless implementation must fail unless inventory generation is explicit");
    assert!(
        err.to_string().contains("expected_target_files"),
        "err={err}"
    );
}

#[test]
fn generated_required_artifacts_do_not_self_heal_without_request() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-required-artifacts
task: Build project artifacts.
stages:
  - id: implementation_report
    kind: agent
  - id: final_quality
    kind: quality_gate
    depends_on: [implementation_report]
    required_artifacts:
      - .archon/trading-lab/strategies/AHDM-v1/strategy-spec.json
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let ids = spec
        .stages
        .iter()
        .map(|stage| stage.id.as_str())
        .collect::<Vec<_>>();
    assert!(
        !ids.contains(&"required-artifact-inventory"),
        "self-heal must be explicit, ids={ids:?}"
    );
    let gate = spec
        .stages
        .iter()
        .find(|stage| stage.id == "final_quality")
        .unwrap();
    assert_eq!(gate.depends_on, vec!["implementation_report"]);
}

#[test]
fn generated_implementation_stage_accepts_loose_target_files_key() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-loose-target-files
task: Implement a known file.
stages:
  - id: implement_known
    kind: implementation
    task: Implement a known file.
    provider_tier: coder
    target_files:
      - crates/example/src/lib.rs
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implement_known")
        .unwrap();
    assert_eq!(stage.kind, archon_workflow::StageKind::Implementation);
    assert_eq!(
        stage.expected_target_files,
        vec!["crates/example/src/lib.rs".to_string()]
    );
    assert_eq!(
        stage.extra.get("required_work_units"),
        Some(&serde_json::json!(["implement_known"]))
    );
}
