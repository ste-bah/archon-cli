//! Invariant 3 — ungated irreversible action.

use super::super::*;
use super::*;

/// build → gate → deploy, with deploy irreversible.
pub(super) fn gated_graph() -> TaskGraph {
    graph(
        vec![
            TaskNode::new("build", NodeRole::Work),
            TaskNode {
                depends_on: vec!["build".into()],
                ..TaskNode::new("ask", NodeRole::Gate(GateKind::Human))
            },
            TaskNode {
                depends_on: vec!["ask".into()],
                permission: PermissionClass::Irreversible,
                ..TaskNode::new("deploy", NodeRole::Tool)
            },
        ],
        GraphBudget::default(),
    )
}

#[test]
fn invariant_3_blocks_an_irreversible_call_whose_gate_has_not_passed() {
    let live = tracker(LiveTopologyConfig::default());
    live.declare_graph(SESSION, &gated_graph());

    let verdict = live.on_tool(
        SESSION,
        &ToolIntent::new("deploy", "Bash", PermissionClass::Irreversible),
    );

    assert_eq!(verdict.invariant(), Some(Invariant::UngatedIrreversible));
    let reason = verdict.reason().expect("blocked carries a reason");
    assert!(reason.contains("deploy"), "names the node: {reason}");
    assert!(reason.contains("ask"), "names the gate to pass: {reason}");
    assert!(
        reason.contains("ungated_irreversible"),
        "names the invariant: {reason}"
    );
}

#[test]
fn invariant_3_admits_the_same_call_once_the_gate_has_passed() {
    let live = tracker(LiveTopologyConfig::default());
    live.declare_graph(SESSION, &gated_graph());

    live.on_gate_passed(SESSION, "ask");

    assert!(
        live.on_tool(
            SESSION,
            &ToolIntent::new("deploy", "Bash", PermissionClass::Irreversible)
        )
        .is_allowed()
    );
}

#[test]
fn invariant_3_admits_a_risky_call_at_the_same_position() {
    // The near-miss on classification rather than on structure.
    let live = tracker(LiveTopologyConfig::default());
    live.declare_graph(SESSION, &gated_graph());

    assert!(
        live.on_tool(
            SESSION,
            &ToolIntent::new("deploy", "Bash", PermissionClass::Risky)
        )
        .is_allowed()
    );
    assert!(
        live.on_tool(
            SESSION,
            &ToolIntent::new("deploy", "Read", PermissionClass::Safe)
        )
        .is_allowed()
    );
}

#[test]
fn invariant_3_does_not_count_a_checkpoint_as_gating() {
    // The tripwire. `StageKind::Checkpoint` has no execution semantics anywhere
    // — nothing marks a checkpoint passed — so a checkpoint present in the
    // graph must not authorise an irreversible action. Legacy `condition`
    // stages deserialize to Checkpoint, and a condition never had an evaluator.
    let live = tracker(LiveTopologyConfig::default());
    live.declare_graph(
        SESSION,
        &graph(
            vec![
                TaskNode::new("check", NodeRole::Gate(GateKind::Checkpoint)),
                TaskNode {
                    depends_on: vec!["check".into()],
                    permission: PermissionClass::Irreversible,
                    ..TaskNode::new("deploy", NodeRole::Tool)
                },
            ],
            GraphBudget::default(),
        ),
    );

    let verdict = live.on_tool(
        SESSION,
        &ToolIntent::new("deploy", "Bash", PermissionClass::Irreversible),
    );

    assert!(
        verdict.is_blocked(),
        "a checkpoint that nothing marks passed must not gate"
    );
}

#[test]
fn invariant_3_defaults_to_not_enforcing_where_no_gate_is_declared() {
    // The design defect, pinned. Read literally, this call is "not dominated by
    // a passed gate" and would be blocked — which under the default config
    // would block every `git push` in every plain session.
    let live = tracker(LiveTopologyConfig::default());
    assert_eq!(
        live.config().ungated_irreversible,
        GateEnforcement::WhereDeclared
    );

    assert!(
        live.on_tool(
            SESSION,
            &ToolIntent::new("root", "Bash", PermissionClass::Irreversible)
        )
        .is_allowed()
    );

    // A declared graph that declares no gates is the same case: the author
    // never opted into gating.
    live.declare_graph(
        SESSION,
        &graph(
            vec![TaskNode::new("root", NodeRole::Work)],
            GraphBudget::default(),
        ),
    );
    assert!(
        live.on_tool(
            SESSION,
            &ToolIntent::new("root", "Bash", PermissionClass::Irreversible)
        )
        .is_allowed()
    );
}

#[test]
fn invariant_3_strict_mode_blocks_an_undeclared_session_too() {
    let live = tracker(LiveTopologyConfig::strict());

    let verdict = live.on_tool(
        SESSION,
        &ToolIntent::new("root", "Bash", PermissionClass::Irreversible),
    );

    assert_eq!(verdict.invariant(), Some(Invariant::UngatedIrreversible));
    assert!(
        verdict
            .reason()
            .expect("reason")
            .contains("no gate has passed in this session")
    );
}

#[test]
fn invariant_3_strict_mode_admits_an_undeclared_session_after_any_gate_passes() {
    // In an undeclared prefix the executed history is a chain, so a gate
    // already passed precedes — and therefore dominates — everything after it.
    let live = tracker(LiveTopologyConfig::strict());
    live.on_gate_passed(SESSION, "human-approval");

    assert!(
        live.on_tool(
            SESSION,
            &ToolIntent::new("root", "Bash", PermissionClass::Irreversible)
        )
        .is_allowed()
    );
}

#[test]
fn invariant_3_can_be_disabled_alone() {
    let live = tracker(LiveTopologyConfig {
        ungated_irreversible: GateEnforcement::Off,
        ..LiveTopologyConfig::strict()
    });
    live.declare_graph(SESSION, &gated_graph());

    assert!(
        live.on_tool(
            SESSION,
            &ToolIntent::new("deploy", "Bash", PermissionClass::Irreversible)
        )
        .is_allowed()
    );
}
