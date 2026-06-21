use archon_workflow::WorkflowSpec;

#[test]
fn generated_remediation_stages_get_noop_contracts() {
    let spec = WorkflowSpec::from_generated_yaml(
        r#"
schema: archon.workflow.v1
name: generated-remediation-contract
task: Implement a repo change.
stages:
  - id: wave1_remediation_inventory
    kind: agent
    task: Return remediation inventory.
  - id: wave1_remediation_impl
    kind: fanout
    task: Apply only required remediation fixes.
    foreach: "${wave1_remediation_inventory.items}"
    depends_on: [wave1_remediation_inventory]
    item_kind: implementation
  - id: wave1_post_remediation_tests
    kind: tool
    task: Run only focused tests required by wave1 remediation items.
    verify_command: "${wave1_remediation_impl.focused_test_command}"
    depends_on: [wave1_remediation_impl]
  - id: wave1_post_remediation_review
    kind: fanout
    task: Read-only post-remediation review.
    foreach: "${wave1_remediation_inventory.items}"
    depends_on:
      - wave1_remediation_impl
      - wave1_post_remediation_tests
      - wave1_remediation_inventory
"#,
        "fallback task",
    )
    .unwrap();

    let inventory = spec
        .stages
        .iter()
        .find(|stage| stage.id == "wave1_remediation_inventory")
        .unwrap();
    assert_eq!(
        inventory
            .extra
            .get("outputs")
            .and_then(|value| value.as_array())
            .and_then(|values| values.first())
            .and_then(|value| value.as_str()),
        Some("items")
    );

    let remediation_impl = spec
        .stages
        .iter()
        .find(|stage| stage.id == "wave1_remediation_impl")
        .unwrap();
    assert_eq!(
        remediation_impl
            .extra
            .get("allow_empty_items")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let tests = spec
        .stages
        .iter()
        .find(|stage| stage.id == "wave1_post_remediation_tests")
        .unwrap();
    assert!(tests.verify_command.is_none());
    assert!(
        tests
            .extra
            .get("removed_unresolved_verify_command")
            .is_some()
    );
    assert_eq!(
        tests
            .extra
            .get("allow_empty_remediation_noop")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let review = spec
        .stages
        .iter()
        .find(|stage| stage.id == "wave1_post_remediation_review")
        .unwrap();
    assert_eq!(
        review
            .extra
            .get("allow_empty_items")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        review
            .extra
            .get("failure_aware")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn generated_remediation_inventory_keeps_existing_items_output_case_insensitively() {
    let spec = WorkflowSpec::from_generated_yaml(
        r#"
schema: archon.workflow.v1
name: generated-remediation-items-case
task: Implement a repo change.
stages:
  - id: remediation_inventory
    kind: agent
    outputs: [Items]
"#,
        "fallback task",
    )
    .unwrap();

    let inventory = spec
        .stages
        .iter()
        .find(|stage| stage.id == "remediation_inventory")
        .unwrap();
    let outputs: Vec<_> = inventory
        .extra
        .get("outputs")
        .and_then(|value| value.as_array())
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect();

    assert_eq!(outputs, vec!["Items"]);
}
