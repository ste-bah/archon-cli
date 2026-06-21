use archon_workflow::{WorkflowError, WorkflowSpec};

fn valid_yaml() -> &'static str {
    r#"
schema: archon.workflow.v1
name: repo-deep-audit
task: Audit this repository deeply.
max_parallelism: 12
max_agents: 200
provider_tiers:
  planner: auto
  critic: auto
  reducer: auto
stages:
  - id: discover
    kind: agent
    agent: codebase-analyzer
    provider_tier: planner
    outputs: [items]
  - id: review
    kind: fanout
    agent: code-reviewer
    foreach: "${discover.items}"
    provider_tier: critic
    depends_on: [discover]
  - id: synthesize
    kind: reduce
    reducer: evidence_weighted_report
    depends_on: [review]
"#
}

#[test]
fn generated_non_fanout_stage_drops_invalid_item_kind() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-invalid-item-kind
task: Review the workflow outputs.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: fanout_review
    kind: agent
    item_kind: implementation
    task: Review the discovered items and summarize risks.
    depends_on: [discover]
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "fanout_review")
        .unwrap();
    assert_eq!(stage.kind, archon_workflow::StageKind::Agent);
    assert_eq!(stage.item_kind, None);
}

#[test]
fn generated_foreach_agent_is_promoted_to_fanout() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-foreach-agent
task: Review each discovered item.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: fanout_review
    kind: agent
    foreach: "${discover.items}"
    task: Review each discovered item.
    provider_tier: critic
    depends_on: [discover]
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "fanout_review")
        .unwrap();
    assert_eq!(stage.kind, archon_workflow::StageKind::Fanout);
    assert_eq!(stage.foreach.as_deref(), Some("${discover.items}"));
}

#[test]
fn generated_double_brace_foreach_is_canonicalized() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-double-brace-foreach
task: Review each discovered item.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: fanout_review
    kind: fanout
    foreach: "${{discover.items}}"
    task: Review each discovered item.
    provider_tier: critic
    depends_on: [discover]
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "fanout_review")
        .unwrap();
    assert_eq!(stage.foreach.as_deref(), Some("${discover.items}"));
}

#[test]
fn generated_foreach_adds_missing_depends_on_edge() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-missing-foreach-dependency
task: Review implementation inventory.
stages:
  - id: wave_01_implementation
    kind: agent
  - id: implementation_inventory
    kind: agent
  - id: wave_01_adversarial_review
    kind: fanout
    foreach: ${implementation_inventory.items}
    depends_on: [wave_01_implementation]
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let inventory = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implementation_inventory")
        .unwrap();
    let review = spec
        .stages
        .iter()
        .find(|stage| stage.id == "wave_01_adversarial_review")
        .unwrap();

    assert!(
        inventory
            .extra
            .get("outputs")
            .and_then(serde_json::Value::as_array)
            .unwrap()
            .iter()
            .any(|value| value.as_str() == Some("items"))
    );
    assert!(
        review
            .depends_on
            .contains(&"implementation_inventory".to_string())
    );
    assert!(
        review
            .depends_on
            .contains(&"wave_01_implementation".to_string())
    );
}

#[test]
fn user_authored_foreach_missing_depends_on_edge_is_rejected() {
    let yaml = r#"
schema: archon.workflow.v1
name: user-missing-foreach-dependency
task: Review implementation inventory.
stages:
  - id: implementation_inventory
    kind: agent
    outputs: [items]
  - id: wave_01_adversarial_review
    kind: fanout
    foreach: ${implementation_inventory.items}
"#;
    let err = WorkflowSpec::from_yaml(yaml).unwrap_err();

    assert!(
        err.to_string().contains(
            "foreach references 'implementation_inventory' which is not in its depends_on"
        )
    );
}

#[test]
fn fanout_filter_is_first_class_and_validated() {
    let yaml = r#"
schema: archon.workflow.v1
name: filtered-fanout
task: Implement only wave1.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: "${discover.items}"
    filter: item.wave_id == 'wave1'
    depends_on: [discover]
"#;
    let spec = WorkflowSpec::from_yaml(yaml).unwrap();
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implement")
        .unwrap();
    assert_eq!(stage.filter.as_deref(), Some("item.wave_id == 'wave1'"));
    assert!(!stage.extra.contains_key("filter"));
}

#[test]
fn malformed_fanout_filter_is_rejected() {
    let yaml = r#"
schema: archon.workflow.v1
name: malformed-filter
task: Implement only wave1.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: "${discover.items}"
    filter: wave_id == wave1
    depends_on: [discover]
"#;
    let err = WorkflowSpec::from_yaml(yaml).unwrap_err();
    assert!(
        matches!(err, WorkflowError::InvalidFanout(_)),
        "got {err:?}"
    );
}

#[test]
fn non_fanout_filter_is_rejected() {
    let yaml = r#"
schema: archon.workflow.v1
name: misplaced-filter
task: Discover only.
stages:
  - id: discover
    kind: agent
    filter: item.wave_id == 'wave1'
"#;
    let err = WorkflowSpec::from_yaml(yaml).unwrap_err();
    assert!(
        err.to_string()
            .contains("filter is only supported on fanout")
    );
}

#[test]
fn generated_missing_kind_is_inferred_before_deserialize() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-missing-kind
task: Implement missing repository work.
stages:
  - id: discover
    outputs: [items]
  - id: wave1_implementation
    task: Implement missing work for T001.
    provider_tier: executor
    expected_target_files: ["src/lib.rs"]
    depends_on: [discover]
  - id: final_synthesis
    reducer: code_review_synthesis
    depends_on: [wave1_implementation]
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let implementation = spec
        .stages
        .iter()
        .find(|stage| stage.id == "wave1_implementation")
        .unwrap();
    let synthesis = spec
        .stages
        .iter()
        .find(|stage| stage.id == "final_synthesis")
        .unwrap();
    assert_eq!(
        implementation.kind,
        archon_workflow::StageKind::Implementation
    );
    assert_eq!(
        implementation.provider_tier,
        Some(archon_workflow::ProviderTier::Coder)
    );
    assert_eq!(synthesis.kind, archon_workflow::StageKind::Reduce);
}

#[test]
fn user_authored_missing_kind_is_rejected() {
    let yaml = r#"
schema: archon.workflow.v1
name: missing-kind
task: Invalid hand-authored workflow.
stages:
  - id: discover
"#;
    let err = WorkflowSpec::from_yaml(yaml).unwrap_err();
    assert!(err.to_string().contains("missing field `kind`"));
}

#[test]
fn user_authored_implementation_fanout_without_item_kind_is_rejected() {
    let yaml = r#"
schema: archon.workflow.v1
name: missing-item-kind
task: Implement the decomposed PRD.
stages:
  - id: task_inventory
    kind: agent
    outputs: [items]
  - id: implement_task
    kind: fanout
    task: Implement only the missing work for each item.
    provider_tier: coder
    foreach: "${task_inventory.items}"
    depends_on: [task_inventory]
"#;
    let err = WorkflowSpec::from_yaml(yaml).unwrap_err();
    assert!(
        err.to_string().contains("item_kind: implementation"),
        "got {err:?}"
    );
}

#[test]
fn inline_implementation_items_require_work_unit_metadata() {
    let yaml = r#"
schema: archon.workflow.v1
name: missing-work-unit-metadata
task: Implement the decomposed PRD.
stages:
  - id: implement_task
    kind: fanout
    item_kind: implementation
    input:
      items:
        - target_files: ["src/lib.rs"]
"#;
    let err = WorkflowSpec::from_yaml(yaml).unwrap_err();
    assert!(
        err.to_string().contains("requires work_unit_id"),
        "got {err:?}"
    );
}

#[test]
fn inline_implementation_items_can_inherit_stage_work_unit_scope() {
    let yaml = r#"
schema: archon.workflow.v1
name: stage-scoped-remediation-items
task: Implement the decomposed PRD.
stages:
  - id: fix_t010
    kind: fanout
    item_kind: implementation
    completion_task_ids: ["TASK-TDL-010"]
    input:
      items:
        - target_files: ["src/data_store.rs"]
          finding: "rewrite migrated metadata"
        - target_files: ["src/data_store/io.rs"]
          finding: "use temp-file write"
"#;
    let spec = WorkflowSpec::from_yaml(yaml).unwrap();
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "fix_t010")
        .unwrap();

    assert_eq!(
        stage.item_kind,
        Some(archon_workflow::StageKind::Implementation)
    );
}

#[test]
fn direct_implementation_stage_requires_work_unit_metadata() {
    let yaml = r#"
schema: archon.workflow.v1
name: direct-missing-work-unit-metadata
task: Implement a known file.
stages:
  - id: implement_known
    kind: implementation
    expected_target_files: ["src/lib.rs"]
"#;
    let err = WorkflowSpec::from_yaml(yaml).unwrap_err();
    assert!(
        err.to_string().contains("requires completion_task_ids"),
        "got {err:?}"
    );
}

#[test]
fn fanout_with_unresolvable_over_token_is_rejected() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-fanout-unresolvable
task: Implement the decomposed PRD.
stages:
  - id: build-dependency-dag
    kind: agent
    provider_tier: planner
  - id: implement-workstreams
    kind: fanout
    provider_tier: coder
    depends_on: [build-dependency-dag]
    fanout:
      over: ordered_workstreams
"#;
    let err = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap_err();
    assert!(
        matches!(err, WorkflowError::InvalidFanout(_)),
        "got {err:?}"
    );
}

#[test]
fn fanout_foreach_without_items_accessor_is_rejected() {
    let bad = valid_yaml().replace("${discover.items}", "${discover.modules}");
    let err = WorkflowSpec::from_yaml(&bad).unwrap_err();
    assert!(
        matches!(err, WorkflowError::InvalidFanout(_)),
        "got {err:?}"
    );
}

#[test]
fn fanout_foreach_producer_without_items_declaration_is_rejected() {
    let bad = valid_yaml().replace("    outputs: [items]\n", "");
    let err = WorkflowSpec::from_yaml(&bad).unwrap_err();
    assert!(
        matches!(err, WorkflowError::InvalidFanout(_)),
        "got {err:?}"
    );
}

#[test]
fn bare_fanout_without_iteration_remains_valid() {
    let yaml = r#"
schema: archon.workflow.v1
name: bare-fanout
task: Single item fanout.
stages:
  - id: discover
    kind: agent
    provider_tier: planner
  - id: review
    kind: fanout
    provider_tier: critic
    depends_on: [discover]
"#;
    let spec = WorkflowSpec::from_yaml(yaml).unwrap();
    assert!(spec.stages[1].foreach.is_none());
}
