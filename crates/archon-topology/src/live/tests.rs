//! Each invariant blocks its violation and admits the near-miss.
//!
//! The near-miss is the half that matters. A check that blocks everything also
//! blocks every violation, so a blocking test alone proves nothing.

mod agent_cap;
mod composition;
mod single_writer;
mod ungated_irreversible;

use super::*;
use crate::ir::{GateKind, GraphBudget, GraphOrigin, NodeRole, PermissionClass, TaskNode};

const SESSION: &str = "s-1";

fn tracker(config: LiveTopologyConfig) -> LiveTopology {
    let live = LiveTopology::new(config);
    assert!(live.begin_session(SESSION));
    live
}

fn spawn(node: &str) -> SpawnIntent {
    SpawnIntent {
        node_id: node.to_string(),
        parent_id: None,
        agent: "worker".to_string(),
    }
}

fn write(node: &str, path: &str) -> WriteIntent {
    WriteIntent {
        node_id: node.to_string(),
        paths: vec![path.to_string()],
        shared_append: Vec::new(),
    }
}

fn graph(nodes: Vec<TaskNode>, budget: GraphBudget) -> TaskGraph {
    TaskGraph {
        id: "g".into(),
        origin: GraphOrigin::Session {
            session_id: SESSION.into(),
        },
        nodes,
        budget,
    }
}

// ---------------------------------------------------------------------------
// Absent LiveTopology admits everything
// ---------------------------------------------------------------------------

#[test]
fn an_untracked_session_admits_everything() {
    let live = LiveTopology::new(LiveTopologyConfig::strict());
    // No begin_session: nothing is tracked.
    assert!(live.on_spawn("nobody", &spawn("a")).is_allowed());
    assert!(
        live.on_write_intent("nobody", &write("a", "src/lib.rs"))
            .is_allowed()
    );
    assert!(
        live.on_tool(
            "nobody",
            &ToolIntent::new("a", "Bash", PermissionClass::Irreversible)
        )
        .is_allowed()
    );
    assert!(!live.tracks("nobody"));
}

#[test]
fn ending_a_session_drops_its_state_and_reverts_to_admitting_everything() {
    let live = tracker(LiveTopologyConfig::strict());
    live.declare_graph(
        SESSION,
        &graph(
            vec![TaskNode::new("a", NodeRole::Work)],
            GraphBudget {
                max_agents: 0,
                ..GraphBudget::default()
            },
        ),
    );
    assert!(live.on_spawn(SESSION, &spawn("child")).is_blocked());

    live.end_session(SESSION);

    assert!(!live.tracks(SESSION));
    assert!(live.on_spawn(SESSION, &spawn("child")).is_allowed());
}

#[test]
fn the_session_map_is_bounded() {
    let live = LiveTopology::new(LiveTopologyConfig::default());
    for index in 0..MAX_TRACKED_SESSIONS {
        assert!(live.begin_session(&format!("s-{index}")), "{index}");
    }
    assert_eq!(live.len(), MAX_TRACKED_SESSIONS);
    // Over the bound the new session is refused, not tracked-and-evicted: an
    // untracked session admits everything, whereas evicting a live one would
    // discard write claims concurrent nodes still rely on.
    assert!(!live.begin_session("one-too-many"));
    assert!(!live.tracks("one-too-many"));
    assert!(live.tracks("s-0"));
}

#[test]
fn beginning_an_already_tracked_session_keeps_its_prefix() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_write_intent(SESSION, &write("a", "src/lib.rs"));
    assert!(!live.begin_session(SESSION));

    // The claim survived, so a conflicting write from an unrelated live node is
    // still caught.
    live.on_node_started(SESSION, "a");
    assert!(
        live.on_write_intent(SESSION, &write("b", "src/lib.rs"))
            .is_blocked()
    );
}
