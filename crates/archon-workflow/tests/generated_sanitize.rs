use archon_workflow::WorkflowSpec;

#[test]
fn generated_yaml_drops_unknown_top_level_fields_and_fills_stage_ids() {
    let yaml = r#"
version: 1
schema: archon.workflow.v1
name: generated-with-missing-ids
task: Implement the decomposed PRD.
inputs:
  prd: /tmp/PRD.md
stages:
  - name: discovery
    kind: agent
    outputs: [items]
  - task: Implement workstreams.
    kind: fanout
    item_kind: implementation
    foreach: ${discovery.items}
    depends_on: [discovery]
"#;

    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();

    assert_eq!(spec.stages[0].id, "discovery");
    assert_eq!(spec.stages[1].id, "implement_workstreams");
    assert!(WorkflowSpec::from_yaml(yaml).is_err());
}

#[test]
fn generated_yaml_accepts_map_shaped_stages() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-map-stages
task: Review the implementation.
stages:
  discovery:
    kind: agent
    outputs: [items]
  review:
    kind: fanout
    foreach: ${discovery.items}
    depends_on: [discovery]
"#;

    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();

    assert_eq!(spec.stages[0].id, "discovery");
    assert_eq!(spec.stages[1].id, "review");
}
