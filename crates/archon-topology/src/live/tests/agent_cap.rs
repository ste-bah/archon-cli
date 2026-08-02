//! Invariant 1 — the lifetime agent cap.

use super::super::*;
use super::*;

#[test]
fn invariant_1_blocks_the_spawn_past_the_lifetime_budget() {
    let live = tracker(LiveTopologyConfig::default());
    live.declare_graph(
        SESSION,
        &graph(
            vec![TaskNode::new("root", NodeRole::Work)],
            GraphBudget {
                max_agents: 2,
                ..GraphBudget::default()
            },
        ),
    );

    assert!(live.on_spawn(SESSION, &spawn("a")).is_allowed());
    assert!(live.on_spawn(SESSION, &spawn("b")).is_allowed());
    let verdict = live.on_spawn(SESSION, &spawn("c"));

    assert_eq!(verdict.invariant(), Some(Invariant::AgentCap));
    let reason = verdict.reason().expect("blocked carries a reason");
    assert!(reason.contains('c'), "names the node: {reason}");
    assert!(
        reason.contains("agent_cap"),
        "names the invariant: {reason}"
    );
    // A blocked spawn must not consume budget it never used.
    assert_eq!(live.snapshot(SESSION).expect("tracked").agents_spawned(), 2);
}

#[test]
fn invariant_1_admits_the_near_miss_at_exactly_the_budget() {
    let live = tracker(LiveTopologyConfig::default());
    live.declare_graph(
        SESSION,
        &graph(
            vec![TaskNode::new("root", NodeRole::Work)],
            GraphBudget {
                max_agents: 2,
                ..GraphBudget::default()
            },
        ),
    );

    assert!(live.on_spawn(SESSION, &spawn("a")).is_allowed());
    assert!(live.on_spawn(SESSION, &spawn("b")).is_allowed());
}

#[test]
fn invariant_1_is_a_lifetime_total_not_a_concurrency_cap() {
    // The O2 defect in one test: AgentPool releases on completion, so finishing
    // an agent used to free budget. It must not.
    let live = tracker(LiveTopologyConfig::default());
    live.declare_graph(
        SESSION,
        &graph(
            vec![TaskNode::new("root", NodeRole::Work)],
            GraphBudget {
                max_agents: 1,
                ..GraphBudget::default()
            },
        ),
    );

    assert!(live.on_spawn(SESSION, &spawn("a")).is_allowed());
    live.on_agent_finished(SESSION, "a");
    assert_eq!(live.snapshot(SESSION).expect("tracked").live_agents(), 0);

    assert!(live.on_spawn(SESSION, &spawn("b")).is_blocked());
}

#[test]
fn a_session_with_no_declared_graph_uses_the_configured_ceiling() {
    // Without this the agent cap would silently enforce the IR default rather
    // than what the operator configured, and `[topology] max_agents` would be
    // dead.
    let live = tracker(LiveTopologyConfig {
        max_agents: 1,
        ..LiveTopologyConfig::default()
    });

    assert!(live.on_spawn(SESSION, &spawn("a")).is_allowed());
    assert!(live.on_spawn(SESSION, &spawn("b")).is_blocked());
}

#[test]
fn a_declared_budget_overrides_the_configured_ceiling() {
    // An authored budget is the stronger statement.
    let live = tracker(LiveTopologyConfig {
        max_agents: 1,
        ..LiveTopologyConfig::default()
    });
    live.declare_graph(
        SESSION,
        &graph(
            vec![TaskNode::new("root", NodeRole::Work)],
            GraphBudget {
                max_agents: 3,
                ..GraphBudget::default()
            },
        ),
    );

    for id in ["a", "b", "c"] {
        assert!(live.on_spawn(SESSION, &spawn(id)).is_allowed(), "{id}");
    }
    assert!(live.on_spawn(SESSION, &spawn("d")).is_blocked());
}

#[test]
fn invariant_1_can_be_disabled_alone() {
    let live = tracker(LiveTopologyConfig {
        agent_cap: false,
        ..LiveTopologyConfig::default()
    });
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

    assert!(live.on_spawn(SESSION, &spawn("a")).is_allowed());
    // Still counted, just not enforced.
    assert_eq!(live.snapshot(SESSION).expect("tracked").agents_spawned(), 1);
}
