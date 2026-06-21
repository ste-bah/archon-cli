use archon_workflow::WorkflowSpec;

#[test]
fn explicit_fix_every_issue_request_adds_remediation_before_each_post_write_gate() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-review-fix-loop
task: Implement a decomposed PRD. After every implementation workstream, run an adversarial review and fix every issue found before moving to the next workstream.
stages:
  - id: t001_inventory
    kind: agent
    outputs: [items]
  - id: t001_implementation
    kind: fanout
    task: Implement only missing work for T001.
    foreach: "${t001_inventory.items}"
    item_kind: implementation
    depends_on: [t001_inventory]
  - id: t001_review
    kind: agent
    provider_tier: critic
    depends_on: [t001_implementation]
  - id: t001_gate
    kind: quality_gate
    depends_on: [t001_inventory, t001_implementation, t001_review]
  - id: t010_inventory
    kind: agent
    outputs: [items]
    depends_on: [t001_gate]
  - id: t010_implementation
    kind: fanout
    task: Implement only missing work for T010.
    foreach: "${t010_inventory.items}"
    item_kind: implementation
    depends_on: [t010_inventory]
  - id: t010_review
    kind: agent
    provider_tier: critic
    depends_on: [t010_implementation]
  - id: t010_gate
    kind: quality_gate
    depends_on: [t010_inventory, t010_implementation, t010_review]
"#;

    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let ids = spec
        .stages
        .iter()
        .map(|stage| stage.id.as_str())
        .collect::<Vec<_>>();

    assert!(ids.contains(&"remediation-inventory"));
    assert!(ids.contains(&"remediation-inventory-2"));
    assert_gate_depends_on_report(&spec, "t001_gate", "post-remediation-acceptance-report");
    assert_gate_depends_on_report(&spec, "t010_gate", "post-remediation-acceptance-report-2");
}

fn assert_gate_depends_on_report(spec: &WorkflowSpec, gate_id: &str, report_id: &str) {
    let gate = spec
        .stages
        .iter()
        .find(|stage| stage.id == gate_id)
        .unwrap();
    assert_eq!(gate.depends_on, vec![report_id.to_string()]);
}
