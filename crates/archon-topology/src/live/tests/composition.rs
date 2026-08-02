//! How the three invariants compose on one tool call, and the bookkeeping
//! around them.

use super::super::*;
use super::ungated_irreversible::gated_graph;
use super::*;

#[test]
fn on_tool_runs_the_write_check_for_paths_the_call_declares() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/lib.rs"))
            .is_allowed()
    );

    let verdict = live.on_tool(
        SESSION,
        &ToolIntent::new("b", "Write", PermissionClass::Risky)
            .with_writes(vec!["src/lib.rs".into()]),
    );

    assert_eq!(verdict.invariant(), Some(Invariant::SingleWriter));
}

#[test]
fn on_tool_runs_the_agent_cap_for_a_spawning_call() {
    let live = tracker(LiveTopologyConfig::default());
    live.declare_graph(
        SESSION,
        &graph(
            vec![TaskNode::new("root", NodeRole::Work)],
            GraphBudget {
                max_agents: 0,
                ..GraphBudget::default()
            },
        ),
    );

    let verdict = live.on_tool(
        SESSION,
        &ToolIntent::new("root", "Agent", PermissionClass::Risky).with_spawn(spawn("child")),
    );

    assert_eq!(verdict.invariant(), Some(Invariant::AgentCap));
}

#[test]
fn a_malformed_graph_is_ignored_rather_than_failing_closed() {
    let live = tracker(LiveTopologyConfig::strict());
    let cyclic = graph(
        vec![
            TaskNode {
                depends_on: vec!["b".into()],
                ..TaskNode::new("a", NodeRole::Work)
            },
            TaskNode {
                depends_on: vec!["a".into()],
                ..TaskNode::new("b", NodeRole::Gate(GateKind::Human))
            },
        ],
        GraphBudget::default(),
    );

    live.declare_graph(SESSION, &cyclic);

    let state = live.snapshot(SESSION).expect("tracked");
    assert!(
        !state.has_declared_graph(),
        "a graph that does not validate must not be adopted"
    );
}

#[test]
fn rounds_are_recorded_but_not_enforced() {
    let live = tracker(LiveTopologyConfig::strict());
    live.declare_graph(
        SESSION,
        &graph(
            vec![TaskNode::new("a", NodeRole::Work)],
            GraphBudget {
                max_rounds: 1,
                ..GraphBudget::default()
            },
        ),
    );

    for _ in 0..5 {
        live.on_round(SESSION, "a");
        assert!(
            live.on_tool(
                SESSION,
                &ToolIntent::new("a", "Bash", PermissionClass::Risky)
            )
            .is_allowed()
        );
    }

    assert_eq!(live.snapshot(SESSION).expect("tracked").round("a"), 5);
}

#[test]
fn declaring_a_graph_keeps_the_executed_prefix() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    live.on_write_intent(SESSION, &write("a", "src/lib.rs"));
    live.on_gate_passed(SESSION, "ask");

    live.declare_graph(SESSION, &gated_graph());

    let state = live.snapshot(SESSION).expect("tracked");
    assert_eq!(state.gates_passed(), vec!["ask".to_string()]);
    // And the claim survived the declaration.
    live.on_node_started(SESSION, "unrelated");
    assert!(
        live.on_write_intent(SESSION, &write("unrelated", "src/lib.rs"))
            .is_blocked()
    );
}
