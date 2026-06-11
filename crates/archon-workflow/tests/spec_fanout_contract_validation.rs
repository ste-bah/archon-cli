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
