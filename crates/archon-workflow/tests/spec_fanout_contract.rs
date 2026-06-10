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
fn fanout_over_token_bridges_to_foreach_when_producer_declares_items() {
    // Reproduces the live failure (run wf-f28e3ce2): the planner emitted a
    // decorative `fanout: {over: ordered_workstreams}` block with foreach=null.
    // The generated normalizer must bridge `over` to a real
    // `foreach: ${producer.items}` when the token resolves to an upstream stage
    // that advertises it via `outputs`, and add the depends_on edge.
    let yaml = r#"
schema: archon.workflow.v1
name: generated-fanout-bridge
task: Implement the decomposed PRD.
stages:
  - id: build-dependency-dag
    kind: agent
    provider_tier: planner
    outputs: [task_dag, ordered_workstreams]
  - id: implement-workstreams
    kind: fanout
    provider_tier: coder
    depends_on: [build-dependency-dag]
    fanout:
      over: ordered_workstreams
      respect_dependencies: task_dag
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let fanout = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implement-workstreams")
        .unwrap();
    assert_eq!(
        fanout.foreach.as_deref(),
        Some("${build-dependency-dag.items}")
    );
    assert!(
        fanout
            .depends_on
            .contains(&"build-dependency-dag".to_string())
    );
    // The bridge also records `items` on the producer so the plan satisfies the
    // producer side of the contract end to end.
    let producer = spec
        .stages
        .iter()
        .find(|stage| stage.id == "build-dependency-dag")
        .unwrap();
    let outputs: Vec<String> = match producer.extra.get("outputs") {
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    };
    assert!(outputs.iter().any(|o| o == "items"), "outputs={outputs:?}");
}

#[test]
fn generated_implementation_fanout_gets_item_kind() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-implementation-fanout
task: Implement the decomposed PRD.
stages:
  - id: task_inventory
    kind: agent
    outputs: [items]
  - id: implement_task
    kind: fanout
    task: Implement only the missing work for each item and modify repository files directly.
    provider_tier: coder
    foreach: "${task_inventory.items}"
    depends_on: [task_inventory]
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implement_task")
        .unwrap();
    assert_eq!(
        stage.item_kind,
        Some(archon_workflow::StageKind::Implementation)
    );
    assert_eq!(
        stage.effective_item_kind(),
        archon_workflow::StageKind::Implementation
    );
}

#[test]
fn generated_write_workflow_gets_remediation_loop_before_quality_gate() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-write-without-repair
task: Implement the decomposed PRD.
stages:
  - id: task_inventory
    kind: agent
    outputs: [items]
  - id: implement_task
    kind: fanout
    task: Implement only the missing work for each item and modify repository files directly.
    provider_tier: coder
    foreach: "${task_inventory.items}"
    depends_on: [task_inventory]
  - id: adversarial_review
    kind: agent
    provider_tier: critic
    depends_on: [implement_task]
  - id: final_synthesis
    kind: reduce
    depends_on: [adversarial_review]
  - id: quality_gate
    kind: quality_gate
    depends_on: [final_synthesis]
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let ids = spec
        .stages
        .iter()
        .map(|stage| stage.id.as_str())
        .collect::<Vec<_>>();
    assert!(
        ids.windows(6).any(|window| {
            window
                == [
                    "remediation-inventory",
                    "remediate-failed-findings",
                    "post-remediation-focused-tests",
                    "post-remediation-adversarial-review",
                    "post-remediation-acceptance-report",
                    "quality_gate",
                ]
        }),
        "ids={ids:?}"
    );
    let repair = spec
        .stages
        .iter()
        .find(|stage| stage.id == "remediate-failed-findings")
        .unwrap();
    assert_eq!(
        repair.foreach.as_deref(),
        Some("${remediation-inventory.items}")
    );
    assert_eq!(
        repair.item_kind,
        Some(archon_workflow::StageKind::Implementation)
    );
    assert_eq!(
        repair
            .extra
            .get("allow_empty_items")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    let inventory = spec
        .stages
        .iter()
        .find(|stage| stage.id == "remediation-inventory")
        .unwrap();
    assert_eq!(
        inventory.depends_on,
        vec![
            "final_synthesis".to_string(),
            "adversarial_review".to_string()
        ]
    );
    let post_review = spec
        .stages
        .iter()
        .find(|stage| stage.id == "post-remediation-adversarial-review")
        .unwrap();
    assert!(
        post_review
            .depends_on
            .contains(&"remediation-inventory".to_string())
    );
    assert!(
        post_review
            .depends_on
            .contains(&"adversarial_review".to_string())
    );
    let gate = spec
        .stages
        .iter()
        .find(|stage| stage.id == "quality_gate")
        .unwrap();
    assert_eq!(
        gate.depends_on,
        vec!["post-remediation-acceptance-report".to_string()]
    );
}

#[test]
fn generated_targetless_implementation_stage_becomes_inventory_fanout() {
    // Reproduces live planner output that emitted `kind: implementation`
    // without `expected_target_files`. Generated plans should not fail
    // validation, but they also must not fake write targets. The safe repair is
    // an inventory stage that emits concrete `target_files`, followed by an
    // implementation fan-out over those items.
    let yaml = r#"
schema: archon.workflow.v1
name: generated-targetless-implementation
task: Implement the decomposed PRD.
stages:
  - id: discover
    kind: agent
  - id: implement_t001
    kind: implementation
    task: Implement TASK-TDL-001.
    provider_tier: coder
    depends_on: [discover]
  - id: focused_tests
    kind: agent
    depends_on: [implement_t001]
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let inventory = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implement_t001-target-inventory")
        .unwrap();
    assert_eq!(inventory.kind, archon_workflow::StageKind::Agent);
    let outputs = inventory
        .extra
        .get("outputs")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        outputs.iter().any(|value| value.as_str() == Some("items")),
        "outputs={outputs:?}"
    );

    let implementation = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implement_t001")
        .unwrap();
    assert_eq!(implementation.kind, archon_workflow::StageKind::Fanout);
    assert_eq!(
        implementation.item_kind,
        Some(archon_workflow::StageKind::Implementation)
    );
    assert_eq!(
        implementation.foreach.as_deref(),
        Some("${implement_t001-target-inventory.items}")
    );
    assert!(
        implementation
            .depends_on
            .contains(&"implement_t001-target-inventory".to_string())
    );
}

#[test]
fn generated_agent_named_implement_becomes_write_capable() {
    // Reproduces a live generated plan where wave implementation stages were
    // emitted as plain agents. They must not execute as read-only/text-only
    // stages; generated normalization promotes them into the same target
    // inventory + implementation fan-out path as targetless implementation
    // stages.
    let yaml = r#"
schema: archon.workflow.v1
name: generated-agent-implementation-wave
task: Implement the decomposed PRD.
stages:
  - id: implementation_plan
    kind: agent
    task: Produce an ordered implementation plan.
  - id: wave1_implement
    kind: agent
    task: Implement only missing T001 work. Run focused T001 tests only.
    depends_on: [implementation_plan]
  - id: wave1_review
    kind: agent
    task: Perform read-only adversarial review for T001.
    depends_on: [wave1_implement]
"#;
    let spec = WorkflowSpec::from_generated_yaml(yaml, "Fallback task").unwrap();
    let inventory = spec
        .stages
        .iter()
        .find(|stage| stage.id == "wave1_implement-target-inventory")
        .unwrap();
    assert_eq!(inventory.kind, archon_workflow::StageKind::Agent);

    let implementation = spec
        .stages
        .iter()
        .find(|stage| stage.id == "wave1_implement")
        .unwrap();
    assert_eq!(implementation.kind, archon_workflow::StageKind::Fanout);
    assert_eq!(
        implementation.item_kind,
        Some(archon_workflow::StageKind::Implementation)
    );
    assert_eq!(
        implementation.foreach.as_deref(),
        Some("${wave1_implement-target-inventory.items}")
    );

    let plan = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implementation_plan")
        .unwrap();
    assert_eq!(plan.kind, archon_workflow::StageKind::Agent);
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
}

#[test]
fn generated_non_fanout_stage_drops_invalid_item_kind() {
    // Reproduces live planner output that attached `item_kind: implementation`
    // to a read-only review/agent stage. User-authored specs stay strict, but
    // generated plans should repair the invalid field instead of aborting.
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
    // A decorative fan-out whose `over` token resolves to no structured-items
    // producer cannot be bridged and must be rejected, not silently collapsed
    // to a single synthetic item at runtime.
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
    // A fan-out must iterate via the `.items` accessor; other accessors are
    // never resolved by the runtime.
    let bad = valid_yaml().replace("${discover.items}", "${discover.modules}");
    let err = WorkflowSpec::from_yaml(&bad).unwrap_err();
    assert!(
        matches!(err, WorkflowError::InvalidFanout(_)),
        "got {err:?}"
    );
}

#[test]
fn fanout_foreach_producer_without_items_declaration_is_rejected() {
    // The producer side of the contract: a foreach source that does not declare
    // `outputs: [items]` (or `produces: items`) is rejected.
    let bad = valid_yaml().replace("    outputs: [items]\n", "");
    let err = WorkflowSpec::from_yaml(&bad).unwrap_err();
    assert!(
        matches!(err, WorkflowError::InvalidFanout(_)),
        "got {err:?}"
    );
}

#[test]
fn bare_fanout_without_iteration_remains_valid() {
    // The single-item case: a fan-out with neither foreach nor decorative
    // iteration keys is still a valid spec.
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
