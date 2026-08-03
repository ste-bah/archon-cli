//! `WorkflowSpec` → `TaskGraph` lowering.

use archon_topology::{DataRef, GateKind, NodeRole, PermissionClass, WriteTarget};
use archon_workflow::lower_workflow::lower_workflow_spec;
use archon_workflow::spec::WorkflowSpec;

fn spec(yaml: &str) -> WorkflowSpec {
    // `from_yaml` validates; these fixtures are all valid specs.
    WorkflowSpec::from_yaml(yaml).expect("fixture should be a valid spec")
}

/// Parsed without validation, for fixtures that are deliberately defective.
fn unvalidated(yaml: &str) -> WorkflowSpec {
    serde_yaml_ng::from_str(yaml).expect("fixture should parse")
}

#[test]
fn stage_kinds_map_to_roles() {
    let graph = lower_workflow_spec(
        &spec(
            r#"
schema: archon.workflow.v1
name: roles
task: fixture task
stages:
  - { id: plan,   kind: agent,          task: t, agent: planner }
  - id: impl
    kind: implementation
    task: t
    agent: coder
    depends_on: [plan]
    expected_target_files: ["src/impl.rs"]
    task_id: T-1
  - { id: check,  kind: quality_gate,   task: t, depends_on: [impl] }
  - { id: save,   kind: checkpoint,     task: t, depends_on: [check] }
  - { id: ask,    kind: human_gate,     task: t, depends_on: [save] }
  - { id: run,    kind: tool,           tool: bash, depends_on: [ask] }
  - { id: maybe,  kind: condition,      condition: "true", depends_on: [run] }
  - { id: fold,   kind: reduce,         task: t, agent: reducer, depends_on: [maybe] }
"#,
        ),
        "run-1",
    );

    let role = |id: &str| graph.node(id).expect("node present").role;
    assert_eq!(role("plan"), NodeRole::Work);
    assert_eq!(role("impl"), NodeRole::Work);
    assert_eq!(role("check"), NodeRole::Verify);
    assert_eq!(role("save"), NodeRole::Gate(GateKind::Checkpoint));
    assert_eq!(role("ask"), NodeRole::Gate(GateKind::Human));
    assert_eq!(role("run"), NodeRole::Tool);
    assert_eq!(role("fold"), NodeRole::Reduce);
    // Finding W6 removed `StageKind::Condition`: the field had no evaluator, so
    // such a stage never branched. A spec persisted before that removal still
    // carries `kind: condition`, which now deserializes to `Checkpoint` —
    // behaviour-preserving for execution, since an unevaluated condition always
    // proceeded, and matching how `bundle.rs` already grouped the two.
    //
    // The consequence lands here: a legacy condition stage is indistinguishable
    // from a real checkpoint, because the discriminating field is erased at
    // deserialize, so it lowers to a checkpoint gate. That is over-permissive
    // for a presence-based dominator check. It is safe only because milestone 3
    // narrows the relation to gates actually *passed* in the executed prefix,
    // and nothing in the tree marks a checkpoint passed — `StageKind::Checkpoint`
    // has no execution semantics at all, it is a reporting label mapped from
    // `WorkflowV2HostMethod::Checkpoint`. So under milestone 3 this errs toward
    // reporting an irreversible action as ungated, which is the fail-safe
    // direction.
    //
    // If a checkpoint ever gains a real "passed" signal, this becomes a live
    // hole and legacy condition stages must be distinguished before then.
    assert_eq!(role("maybe"), NodeRole::Gate(GateKind::Checkpoint));
}

#[test]
fn foreach_lowers_to_the_only_dataflow_that_exists() {
    let graph = lower_workflow_spec(
        &spec(
            r#"
schema: archon.workflow.v1
name: fanout
task: fixture task
stages:
  - { id: producer, kind: agent, task: enumerate, agent: planner, outputs: [items] }
  - id: worker
    kind: fanout
    task: process
    agent: coder
    depends_on: [producer]
    foreach: "${producer.items}"
    max_parallelism: 3
"#,
        ),
        "run-1",
    );

    let worker = graph.node("worker").expect("node present");
    assert_eq!(worker.role, NodeRole::Work);
    assert_eq!(worker.consumes, vec![DataRef::new("producer", "items")]);
    assert!(worker.dataflow_is_known());

    let fanout = worker.fanout.as_ref().expect("fanout spec");
    assert_eq!(fanout.source, Some(DataRef::new("producer", "items")));
    assert_eq!(fanout.max_parallelism, Some(3));

    // Everything else lowers with `consumes` empty, i.e. dataflow unknown.
    let producer = graph.node("producer").expect("node present");
    assert!(producer.consumes.is_empty());
    assert!(!producer.dataflow_is_known());
    assert!(producer.fanout.is_none());
    assert!(!graph.dataflow_is_complete());
}

#[test]
fn inline_fanout_items_lower_to_a_sourceless_fanout() {
    // A literal item list is a complete, self-contained source with no
    // producer to point at, and WorkflowSpec::validate accepts it.
    let graph = lower_workflow_spec(
        &spec(
            r#"
schema: archon.workflow.v1
name: inline
task: fixture task
stages:
  - id: worker
    kind: fanout
    task: process
    agent: coder
    input: { items: ["a", "b"] }
"#,
        ),
        "run-1",
    );
    let fanout = graph
        .node("worker")
        .expect("node present")
        .fanout
        .as_ref()
        .expect("fanout spec");
    assert!(fanout.source.is_none());
}

#[test]
fn expected_target_files_lower_to_writes() {
    let graph = lower_workflow_spec(
        &spec(
            r#"
schema: archon.workflow.v1
name: writes
task: fixture task
stages:
  - id: edit
    kind: implementation
    task: t
    agent: coder
    expected_target_files: ["src/lib.rs", "src/main.rs"]
    task_id: T-1
  - { id: plain, kind: agent, task: t, agent: reviewer }
"#,
        ),
        "run-1",
    );

    let edit = graph.node("edit").expect("node present");
    assert_eq!(
        edit.writes,
        vec![
            WriteTarget::Path("src/lib.rs".into()),
            WriteTarget::Path("src/main.rs".into()),
        ]
    );
    assert!(edit.writes_are_known());
    assert!(
        !graph
            .node("plain")
            .expect("node present")
            .writes_are_known()
    );
}

#[test]
fn permissions_lower_per_stage_with_a_default_fallback() {
    let graph = lower_workflow_spec(
        &spec(
            r#"
schema: archon.workflow.v1
name: perms
task: fixture task
permissions:
  default: risky
  deploy: irreversible
  review: { class: safe }
  bogus: 17
stages:
  - { id: deploy, kind: tool,  tool: bash,  task: t }
  - { id: review, kind: agent, task: t, agent: reviewer }
  - { id: bogus,  kind: agent, task: t, agent: coder }
  - { id: other,  kind: agent, task: t, agent: coder }
"#,
        ),
        "run-1",
    );

    let class = |id: &str| graph.node(id).expect("node present").permission;
    assert_eq!(class("deploy"), PermissionClass::Irreversible);
    assert_eq!(class("review"), PermissionClass::Safe);
    // An unreadable value falls through to Safe rather than to the `default`
    // entry — fail-open, per the milestone 3 rule that enforcement must never
    // fail closed on a bookkeeping gap.
    assert_eq!(class("bogus"), PermissionClass::Safe);
    assert_eq!(class("other"), PermissionClass::Risky);
}

#[test]
fn budget_comes_from_the_spec() {
    let graph = lower_workflow_spec(
        &spec(
            r#"
schema: archon.workflow.v1
name: budget
task: fixture task
max_parallelism: 4
max_agents: 12
stages:
  - { id: a, kind: agent, task: t, agent: coder }
"#,
        ),
        "run-7",
    );
    assert_eq!(graph.budget.max_parallelism, 4);
    assert_eq!(graph.budget.max_agents, 12);
    // WorkflowSpec has no loop construct.
    assert_eq!(graph.budget.max_rounds, 1);
    assert_eq!(graph.id, "run-7");
}

#[test]
fn lowered_graph_agrees_with_spec_validate_on_cycles() {
    let yaml = r#"
schema: archon.workflow.v1
name: cyclic
task: fixture task
stages:
  - { id: a, kind: agent, task: t, agent: coder, depends_on: [b] }
  - { id: b, kind: agent, task: t, agent: coder, depends_on: [a] }
"#;
    let spec = unvalidated(yaml);
    assert!(spec.validate().is_err(), "spec validate rejects the cycle");
    // The IR must reject it too, rather than silently producing a graph.
    assert!(lower_workflow_spec(&spec, "run-1").waves().is_err());
}

#[test]
fn lowered_graph_agrees_with_spec_validate_on_unknown_dependencies() {
    let yaml = r#"
schema: archon.workflow.v1
name: dangling
task: fixture task
stages:
  - { id: a, kind: agent, task: t, agent: coder, depends_on: [ghost] }
"#;
    let spec = unvalidated(yaml);
    assert!(spec.validate().is_err());
    assert!(lower_workflow_spec(&spec, "run-1").waves().is_err());
}

#[test]
fn a_valid_spec_lowers_to_a_valid_graph() {
    let graph = lower_workflow_spec(
        &spec(
            r#"
schema: archon.workflow.v1
name: diamond
task: fixture task
stages:
  - { id: plan,  kind: agent, task: t, agent: planner }
  - { id: left,  kind: agent, task: t, agent: coder, depends_on: [plan] }
  - { id: right, kind: agent, task: t, agent: coder, depends_on: [plan] }
  - { id: fold,  kind: reduce, task: t, agent: reducer, depends_on: [left, right] }
"#,
        ),
        "run-1",
    );
    assert!(graph.validate().is_ok());
    assert_eq!(
        graph.waves().expect("valid dag"),
        vec![vec!["plan"], vec!["left", "right"], vec!["fold"]]
    );
    assert_eq!(graph.critical_path().expect("valid dag").span(), 3);
}

#[test]
fn workflow_writes_make_conflict_detection_immediately_meaningful() {
    // Two independent implementation stages declaring the same target file.
    let graph = lower_workflow_spec(
        &spec(
            r#"
schema: archon.workflow.v1
name: clash
task: fixture task
stages:
  - { id: plan, kind: agent, task: t, agent: planner }
  - id: left
    kind: implementation
    task: t
    agent: coder
    depends_on: [plan]
    expected_target_files: ["src/lib.rs"]
    task_id: T-1
  - id: right
    kind: implementation
    task: t
    agent: coder
    depends_on: [plan]
    expected_target_files: ["src/lib.rs"]
    task_id: T-2
"#,
        ),
        "run-1",
    );
    let conflicts = graph.write_conflicts().expect("valid dag");
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].left, "left");
    assert_eq!(conflicts[0].right, "right");
}

#[test]
fn human_gate_dominance_carries_through_the_lowering() {
    let graph = lower_workflow_spec(
        &spec(
            r#"
schema: archon.workflow.v1
name: gated
task: fixture task
permissions:
  deploy: irreversible
stages:
  - id: build
    kind: implementation
    task: t
    agent: coder
    expected_target_files: ["src/lib.rs"]
    task_id: T-1
  - { id: ask,    kind: human_gate,     task: t, depends_on: [build] }
  - { id: deploy, kind: tool, tool: bash, task: t, depends_on: [ask] }
"#,
        ),
        "run-1",
    );
    assert!(graph.ungated_irreversible().expect("valid dag").is_empty());
    assert_eq!(graph.gate_nodes(), vec!["ask"]);
}
