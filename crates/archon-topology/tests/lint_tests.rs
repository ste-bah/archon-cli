//! Milestone 4: the three advisory lints.
//!
//! The property every one of these has to hold is *silence on unknown*. Two of
//! the three reason from dataflow, and the majority of graphs in this tree
//! declare none — the `Vec<Subtask>` lowering has nothing to give and a lowered
//! `WorkflowSpec` declares writes but never reads. A lint that treated "not
//! declared" as "declared empty" would report every edge of every team graph as
//! fake, which is the one failure mode that would make the suite worth turning
//! off.

use archon_topology::ir::{
    DataRef, FanoutSpec, GateKind, GraphOrigin, NodeRole, TaskGraph, TaskNode, WriteTarget,
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

// ------------------------------------------------------------- fake edges

#[test]
fn an_edge_whose_dependent_reads_what_the_dependency_writes_is_real() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &["out.json"]),
        node("b", NodeRole::Work, &["a"], &["out.json"], &["final.json"]),
    ]);
    assert!(g.fake_edges().expect("valid").is_empty());
}

#[test]
fn an_edge_with_no_overlapping_dataflow_is_reported_with_both_sides() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &["out.json"]),
        node(
            "b",
            NodeRole::Work,
            &["a"],
            &["other.json"],
            &["final.json"],
        ),
    ]);
    let found = g.fake_edges().expect("valid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].dependent, "b");
    assert_eq!(found[0].dependency, "a");
    assert_eq!(found[0].produced, vec![path("out.json")]);
    assert_eq!(found[0].consumed, vec![path("other.json")]);
    assert!(found[0].remedy().contains("depends_on"));
}

/// The rule the whole suite rests on. `b` declares no consumption at all, so
/// nothing can be concluded about its edge — not "it is fake", not "it is real".
#[test]
fn an_edge_is_never_reported_when_the_dependent_declares_no_consumption() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &["out.json"]),
        node("b", NodeRole::Work, &["a"], &[], &[]),
    ]);
    assert!(g.fake_edges().expect("valid").is_empty());
}

#[test]
fn an_edge_is_never_reported_when_the_dependency_declares_no_production() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &[]),
        node("b", NodeRole::Work, &["a"], &["other.json"], &[]),
    ]);
    assert!(g.fake_edges().expect("valid").is_empty());
}

/// `consumes` is producer-keyed rather than resource-keyed, so a fan-out's
/// `foreach` source justifies its edge on its own without any path overlap.
#[test]
fn a_resolved_producer_reference_justifies_an_edge_without_path_overlap() {
    let g = graph(vec![
        node("producer", NodeRole::Work, &[], &[], &["items.json"]),
        TaskNode {
            depends_on: vec!["producer".to_string()],
            consumes: vec![DataRef::new("producer", "items")],
            reads: vec![path("unrelated.json")],
            ..TaskNode::new("wave", NodeRole::Work)
        },
    ]);
    assert!(g.fake_edges().expect("valid").is_empty());
}

#[test]
fn fake_edges_rejects_a_structurally_invalid_graph_rather_than_guessing() {
    let g = graph(vec![node(
        "b",
        NodeRole::Work,
        &["missing"],
        &["x"],
        &["y"],
    )]);
    assert!(g.fake_edges().is_err(), "an unknown dependency is an error");
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
    let _: Vec<archon_topology::FakeEdge> = g.fake_edges().expect("valid");
    let _: archon_topology::DiamondReport = g.diamond_conformance().expect("valid");
    let _: archon_topology::FusionReport = g.stop_rule_fusion().expect("valid");
    // The graph is otherwise usable: a lint finding changes nothing about it.
    assert_eq!(g.waves().expect("valid").len(), 2);
}
