//! Milestone 4: stop-rule fusion and diamond conformance.
//!
//! The third lint — dependency-edge classification — has its own file,
//! `edge_support_tests.rs`.
//!
//! The property every one of these has to hold is *silence on unknown*. Two of
//! the three reason from dataflow, and the majority of graphs in this tree
//! declare none — the `Vec<Subtask>` lowering has nothing to give and a lowered
//! `WorkflowSpec` declares writes but never reads. A lint that treated "not
//! declared" as "declared empty" would report every edge of every team graph as
//! unsupported, which is the one failure mode that would make the suite worth
//! turning off.

use archon_topology::ir::{
    FanoutSpec, GateKind, GraphOrigin, NodeRole, TaskGraph, TaskNode, WriteTarget,
};
use archon_topology::{DiamondFinding, FusionKind};

fn graph(nodes: Vec<TaskNode>) -> TaskGraph {
    TaskGraph {
        nodes,
        ..TaskGraph::new(
            "g",
            GraphOrigin::Session {
                session_id: "s".to_string(),
            },
        )
    }
}

fn path(value: &str) -> WriteTarget {
    WriteTarget::Path(value.to_string())
}

fn node(
    id: &str,
    role: NodeRole,
    depends_on: &[&str],
    reads: &[&str],
    writes: &[&str],
) -> TaskNode {
    TaskNode {
        depends_on: depends_on.iter().map(|id| (*id).to_string()).collect(),
        reads: reads.iter().map(|value| path(value)).collect(),
        writes: writes.iter().map(|value| path(value)).collect(),
        ..TaskNode::new(id, role)
    }
}

// --------------------------------------------------------- stop-rule fusion

#[test]
fn two_concurrent_nodes_where_one_reads_what_the_other_writes_are_coupled() {
    let g = graph(vec![
        node("root", NodeRole::Work, &[], &[], &["seed.json"]),
        node(
            "left",
            NodeRole::Work,
            &["root"],
            &["seed.json"],
            &["mid.json"],
        ),
        node(
            "right",
            NodeRole::Work,
            &["root"],
            &["mid.json"],
            &["out.json"],
        ),
    ]);
    let report = g.stop_rule_fusion().expect("valid");
    assert_eq!(report.coupled.len(), 1, "{:?}", report.coupled);
    assert_eq!(report.coupled[0].reader, "right");
    assert_eq!(report.coupled[0].writer, "left");
    assert_eq!(report.coupled[0].targets, vec![path("mid.json")]);
    assert_eq!(report.coupled[0].fanout, None);
}

#[test]
fn coupled_fanout_branches_name_the_fanout_that_should_not_be_parallel() {
    let g = graph(vec![
        TaskNode {
            fanout: Some(FanoutSpec {
                source: None,
                max_parallelism: None,
            }),
            writes: vec![path("seed.json")],
            ..TaskNode::new("wave", NodeRole::Work)
        },
        node(
            "item-1",
            NodeRole::Work,
            &["wave"],
            &["seed.json"],
            &["a.json"],
        ),
        node(
            "item-2",
            NodeRole::Work,
            &["wave"],
            &["a.json"],
            &["b.json"],
        ),
    ]);
    let report = g.stop_rule_fusion().expect("valid");
    let coupled = report
        .coupled
        .iter()
        .find(|pair| pair.reader == "item-2")
        .expect("item-2 reads what item-1 writes");
    assert_eq!(coupled.writer, "item-1");
    assert_eq!(coupled.fanout.as_deref(), Some("wave"));
    assert!(coupled.remedy().contains("not parallel work"));
}

/// Sequenced nodes cannot race, however much their targets overlap.
#[test]
fn an_ordered_read_after_write_is_not_coupling() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &["x.json"]),
        node("b", NodeRole::Work, &["a"], &["x.json"], &["y.json"]),
    ]);
    assert!(g.stop_rule_fusion().expect("valid").coupled.is_empty());
}

#[test]
fn a_degenerate_chain_with_the_same_role_and_agent_is_fusible() {
    let g = graph(vec![
        TaskNode {
            agent: Some("coder".to_string()),
            writes: vec![path("a.json")],
            ..TaskNode::new("first", NodeRole::Work)
        },
        TaskNode {
            depends_on: vec!["first".to_string()],
            agent: Some("coder".to_string()),
            reads: vec![path("elsewhere.json")],
            ..TaskNode::new("second", NodeRole::Work)
        },
    ]);
    let report = g.stop_rule_fusion().expect("valid");
    assert_eq!(report.fusible.len(), 1);
    assert_eq!(report.fusible[0].kind, FusionKind::Fuse);
    assert!(report.fusible[0].remedy().contains("merge them"));
}

#[test]
fn a_degenerate_chain_with_different_agents_is_parallelisable() {
    let g = graph(vec![
        TaskNode {
            agent: Some("coder".to_string()),
            writes: vec![path("a.json")],
            ..TaskNode::new("first", NodeRole::Work)
        },
        TaskNode {
            depends_on: vec!["first".to_string()],
            agent: Some("reviewer".to_string()),
            reads: vec![path("elsewhere.json")],
            ..TaskNode::new("second", NodeRole::Verify)
        },
    ]);
    let report = g.stop_rule_fusion().expect("valid");
    assert_eq!(report.fusible[0].kind, FusionKind::Parallelise);
}

#[test]
fn a_chain_carrying_real_dataflow_is_not_fusible() {
    let g = graph(vec![
        node("first", NodeRole::Work, &[], &[], &["a.json"]),
        node("second", NodeRole::Work, &["first"], &["a.json"], &[]),
    ]);
    assert!(g.stop_rule_fusion().expect("valid").fusible.is_empty());
}

#[test]
fn fusion_is_silent_when_neither_side_declares_anything() {
    let g = graph(vec![
        node("first", NodeRole::Work, &[], &[], &[]),
        node("second", NodeRole::Work, &["first"], &[], &[]),
        node("third", NodeRole::Work, &[], &[], &[]),
    ]);
    assert!(g.stop_rule_fusion().expect("valid").is_clean());
}

// ------------------------------------------------------ diamond conformance

#[test]
fn a_verified_fanout_reduce_is_clean() {
    let g = graph(vec![
        TaskNode {
            fanout: Some(FanoutSpec {
                source: None,
                max_parallelism: None,
            }),
            ..TaskNode::new("wave", NodeRole::Work)
        },
        TaskNode {
            depends_on: vec!["wave".to_string()],
            agent: Some("checker".to_string()),
            ..TaskNode::new("verify-a", NodeRole::Verify)
        },
        TaskNode {
            depends_on: vec!["wave".to_string()],
            agent: Some("auditor".to_string()),
            ..TaskNode::new("verify-b", NodeRole::Verify)
        },
        TaskNode {
            depends_on: vec!["verify-a".to_string(), "verify-b".to_string()],
            ..TaskNode::new("fold", NodeRole::Reduce)
        },
    ]);
    let report = g.diamond_conformance().expect("valid");
    assert!(report.is_clean(), "{:?}", report.findings);
    assert_eq!(report.diversity[0].distinct_agents, 2);
}

#[test]
fn several_verifiers_naming_one_agent_are_the_same_reviewer_repeated() {
    let g = graph(vec![
        TaskNode {
            fanout: Some(FanoutSpec {
                source: None,
                max_parallelism: None,
            }),
            ..TaskNode::new("wave", NodeRole::Work)
        },
        TaskNode {
            depends_on: vec!["wave".to_string()],
            agent: Some("checker".to_string()),
            ..TaskNode::new("verify-a", NodeRole::Verify)
        },
        TaskNode {
            depends_on: vec!["wave".to_string()],
            agent: Some("checker".to_string()),
            ..TaskNode::new("verify-b", NodeRole::Verify)
        },
        TaskNode {
            depends_on: vec!["verify-a".to_string(), "verify-b".to_string()],
            ..TaskNode::new("fold", NodeRole::Reduce)
        },
    ]);
    let report = g.diamond_conformance().expect("valid");
    assert_eq!(
        report.findings,
        vec![DiamondFinding::HomogeneousVerifiers {
            reducer: "fold".to_string(),
            verifiers: vec!["verify-a".to_string(), "verify-b".to_string()],
            agent: "checker".to_string(),
        }]
    );
    assert_eq!(report.diversity[0].distinct_agents, 1);
}

/// Two verifiers that never named an agent are *unknown*, not identical.
#[test]
fn verifiers_with_no_declared_agent_are_scored_but_never_flagged() {
    let g = graph(vec![
        TaskNode {
            fanout: Some(FanoutSpec {
                source: None,
                max_parallelism: None,
            }),
            ..TaskNode::new("wave", NodeRole::Work)
        },
        TaskNode {
            depends_on: vec!["wave".to_string()],
            ..TaskNode::new("verify-a", NodeRole::Verify)
        },
        TaskNode {
            depends_on: vec!["wave".to_string()],
            ..TaskNode::new("verify-b", NodeRole::Verify)
        },
        TaskNode {
            depends_on: vec!["verify-a".to_string(), "verify-b".to_string()],
            ..TaskNode::new("fold", NodeRole::Reduce)
        },
    ]);
    let report = g.diamond_conformance().expect("valid");
    assert!(report.is_clean(), "{:?}", report.findings);
    assert_eq!(report.diversity[0].verifiers.len(), 2);
    assert_eq!(report.diversity[0].distinct_agents, 0, "unknown, not one");
}

/// A verifier behind an earlier reduce belongs to that reduce. Counting it here
/// would credit the second fold with verification it never received.
#[test]
fn verification_behind_an_earlier_reduce_is_not_credited_to_the_later_one() {
    let g = graph(vec![
        TaskNode {
            fanout: Some(FanoutSpec {
                source: None,
                max_parallelism: None,
            }),
            ..TaskNode::new("wave", NodeRole::Work)
        },
        TaskNode {
            depends_on: vec!["wave".to_string()],
            agent: Some("checker".to_string()),
            ..TaskNode::new("verify", NodeRole::Verify)
        },
        TaskNode {
            depends_on: vec!["verify".to_string()],
            ..TaskNode::new("fold-1", NodeRole::Reduce)
        },
        TaskNode {
            depends_on: vec!["fold-1".to_string()],
            ..TaskNode::new("fold-2", NodeRole::Reduce)
        },
    ]);
    let report = g.diamond_conformance().expect("valid");
    assert!(
        report
            .diversity
            .iter()
            .all(|score| score.reducer != "fold-2"),
        "fold-2 has no verification of its own"
    );
    assert_eq!(
        report.findings,
        vec![DiamondFinding::SoleVerifier {
            reducer: "fold-1".to_string(),
            verifier: "verify".to_string(),
        }]
    );
}

/// The nearest reducer is the one that folds the fan-out. Reporting every
/// reducer downstream would restate one defect once per later stage.
#[test]
fn only_the_reduce_that_folds_the_fanout_is_checked_for_verification() {
    let g = graph(vec![
        TaskNode {
            fanout: Some(FanoutSpec {
                source: None,
                max_parallelism: None,
            }),
            ..TaskNode::new("wave", NodeRole::Work)
        },
        TaskNode {
            depends_on: vec!["wave".to_string()],
            ..TaskNode::new("fold-1", NodeRole::Reduce)
        },
        TaskNode {
            depends_on: vec!["fold-1".to_string()],
            ..TaskNode::new("fold-2", NodeRole::Reduce)
        },
    ]);
    let report = g.diamond_conformance().expect("valid");
    assert_eq!(
        report.findings,
        vec![DiamondFinding::UnverifiedFanout {
            fanout: "wave".to_string(),
            reducer: "fold-1".to_string(),
        }]
    );
}

#[test]
fn a_graph_with_no_fanout_and_no_reduce_reports_nothing() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &[]),
        TaskNode {
            depends_on: vec!["a".to_string()],
            ..TaskNode::new("gate", NodeRole::Gate(GateKind::Human))
        },
    ]);
    let report = g.diamond_conformance().expect("valid");
    assert!(report.is_clean());
    assert!(report.diversity.is_empty());
}

// ---------------------------------------------------- none of them enforce

/// Every lint returns findings; none of them has a path to a blocking verdict.
/// The `live` module owns enforcement and this suite must never grow one.
#[test]
fn the_lints_return_findings_and_never_a_verdict() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &["out.json"]),
        node("b", NodeRole::Work, &["a"], &["other.json"], &[]),
    ]);
    let _: Vec<archon_topology::ClassifiedEdge> = g.classify_edges().expect("valid");
    let _: archon_topology::DiamondReport = g.diamond_conformance().expect("valid");
    let _: archon_topology::FusionReport = g.stop_rule_fusion().expect("valid");
    // The graph is otherwise usable: a lint finding changes nothing about it.
    assert_eq!(g.waves().expect("valid").len(), 2);
}
