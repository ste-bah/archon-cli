//! The pure derivations, asserted without a database.
//!
//! Everything here exercises [`super::super::derive`] and the two shape
//! statistics in [`super::super::learning_summary`] directly, plus the event
//! type constant those rows are written under.

use archon_topology::ir::{GraphOrigin, NodeRole, TaskGraph, TaskNode};
use archon_topology::trace::TraceReadout;

use super::super::derive::{derive, node_outcomes};
use super::super::learning_summary::{
    TOPOLOGY_OUTCOME_EVENT_TYPE, fan_out_widths, verifier_independence,
};
use super::*;

#[test]
fn the_event_type_matches_the_relations_pascal_case_convention() {
    assert_eq!(
        archon_learning::models::LearningEventType::TopologyOutcome.as_str(),
        TOPOLOGY_OUTCOME_EVENT_TYPE
    );
    assert_eq!(
        archon_learning::models::LearningEventType::from_str(TOPOLOGY_OUTCOME_EVENT_TYPE),
        Some(archon_learning::models::LearningEventType::TopologyOutcome),
        "the design's snake_case spelling would not round-trip"
    );
}

#[test]
fn derived_metrics_are_pure_and_repeatable() {
    let readout = TraceReadout {
        records: vec![
            TraceRecord::new("t", "g1", TraceKind::AgentSpawned)
                .with_node("a")
                .with_parent("turn"),
            TraceRecord::new("t", "g1", TraceKind::AgentSpawned)
                .with_node("b")
                .with_parent("turn"),
        ],
        malformed_lines: 0,
        truncated_tail: false,
    };

    let origin = || GraphOrigin::Session {
        session_id: "s1".into(),
    };
    let first = derive("g1", origin(), None, &readout, "refactor the coordinator");
    let second = derive("g1", origin(), None, &readout, "refactor the coordinator");

    assert_eq!(first.task_hash, second.task_hash);
    assert_eq!(first.span, second.span);
    assert_eq!(first.node_count, 3);
    assert_eq!(first.max_parallelism_used, 2);
    assert!(first.task_hash.starts_with("refactor:"));
}

#[test]
fn a_terminal_failure_is_sticky_across_a_later_retry() {
    let records = vec![
        TraceRecord::new("t", "g", TraceKind::NodeFinished)
            .with_node("a")
            .with_outcome(false, true),
        TraceRecord::new("t", "g", TraceKind::NodeStarted).with_node("a"),
    ];
    assert_eq!(
        node_outcomes(&records).get("a").map(String::as_str),
        Some("failed")
    );
}

#[test]
fn fan_out_widths_report_only_real_fan_outs() {
    let mut graph = TaskGraph::new(
        "g",
        GraphOrigin::Session {
            session_id: "s".into(),
        },
    );
    graph.nodes.push(TaskNode::new("root", NodeRole::Work));
    for id in ["a", "b", "c"] {
        let mut node = TaskNode::new(id, NodeRole::Work);
        node.depends_on = vec!["root".into()];
        graph.nodes.push(node);
    }
    let mut tail = TaskNode::new("tail", NodeRole::Work);
    tail.depends_on = vec!["a".into()];
    graph.nodes.push(tail);

    // `root` fans out to three; `a` has a single dependent and is not a fan-out.
    assert_eq!(fan_out_widths(&graph), vec![3]);
}

#[test]
fn verifier_independence_counts_only_disjoint_verifiers() {
    let mut graph = TaskGraph::new(
        "g",
        GraphOrigin::Session {
            session_id: "s".into(),
        },
    );
    graph.nodes.push(TaskNode::new("p", NodeRole::Work));
    graph.nodes.push(TaskNode::new("q", NodeRole::Work));

    let mut shared_a = TaskNode::new("v1", NodeRole::Verify);
    shared_a.depends_on = vec!["p".into()];
    let mut shared_b = TaskNode::new("v2", NodeRole::Verify);
    shared_b.depends_on = vec!["p".into()];
    let mut disjoint = TaskNode::new("v3", NodeRole::Verify);
    disjoint.depends_on = vec!["q".into()];
    graph.nodes.extend([shared_a, shared_b, disjoint]);

    assert_eq!(
        verifier_independence(&graph),
        1,
        "two verifiers behind the same producer are one check, not two"
    );
}
