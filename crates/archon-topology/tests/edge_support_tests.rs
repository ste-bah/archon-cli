//! Milestone 4: classifying a declared dependency edge.
//!
//! Three answers, not two. The property the suite exists to hold is that
//! *silence on unknown* survives the extra class, and that the class added to
//! stop over-reporting — `EdgeSupport::OrderingOnly` — cannot be reached by an
//! edge that is really an under-declared write.

use archon_topology::ir::{DataRef, GraphOrigin, NodeRole, TaskGraph, TaskNode, WriteTarget};
use archon_topology::{EdgeSupport, LikelyCause};

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

fn artifact(value: &str) -> WriteTarget {
    WriteTarget::Artifact(value.to_string())
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

/// A node whose production and consumption are artifacts rather than files.
fn artifact_node(id: &str, depends_on: &[&str], reads: &[&str], writes: &[&str]) -> TaskNode {
    TaskNode {
        depends_on: depends_on.iter().map(|id| (*id).to_string()).collect(),
        reads: reads.iter().map(|value| artifact(value)).collect(),
        writes: writes.iter().map(|value| artifact(value)).collect(),
        ..TaskNode::new(id, NodeRole::Work)
    }
}

fn support(graph: &TaskGraph, dependent: &str, dependency: &str) -> Option<EdgeSupport> {
    graph
        .classify_edges()
        .expect("valid")
        .into_iter()
        .find(|edge| edge.dependent == dependent && edge.dependency == dependency)
        .map(|edge| edge.support)
}

#[test]
fn an_edge_whose_dependent_reads_what_the_dependency_writes_carries_dataflow() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &["out.json"]),
        node("b", NodeRole::Work, &["a"], &["out.json"], &["final.json"]),
    ]);
    assert_eq!(support(&g, "b", "a"), Some(EdgeSupport::Dataflow));
    assert!(g.unsupported_edges().expect("valid").is_empty());
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
    let found = g.unsupported_edges().expect("valid");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].dependent, "b");
    assert_eq!(found[0].dependency, "a");
    assert_eq!(found[0].produced, vec![path("out.json")]);
    assert_eq!(found[0].consumed, vec![path("other.json")]);
    assert!(found[0].is_defect());
}

/// The remedy must not read as "the dependent is at fault". Both causes are
/// named, always, because on a real corpus the producer was the one that was
/// wrong and dropping the edge would have been the destructive repair.
#[test]
fn an_unsupported_remedy_names_the_producer_cause_as_well_as_the_edge_cause() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &["out.json"]),
        node("b", NodeRole::Work, &["a"], &["other.json"], &[]),
    ]);
    let remedy = g.unsupported_edges().expect("valid")[0].remedy();
    assert!(remedy.contains("depends_on"), "{remedy}");
    assert!(remedy.contains("under-declare"), "{remedy}");
    assert!(remedy.contains('a') && remedy.contains('b'), "{remedy}");
}

/// The producer-side signal. `a` is contracted to produce an artifact nothing
/// in the graph names, and two tasks queue behind it: the ordering is real and
/// the output carrying it was never declared.
#[test]
fn a_producer_nobody_consumes_is_called_out_as_the_likely_cause() {
    let g = graph(vec![
        artifact_node("a", &[], &[], &["registry.json"]),
        artifact_node("b", &["a"], &["other.json"], &[]),
        artifact_node("c", &["a"], &["other.json"], &[]),
    ]);
    assert_eq!(
        support(&g, "b", "a"),
        Some(EdgeSupport::Unsupported(
            LikelyCause::UnderDeclaredProducer {
                dependents: 2,
                artifacts: 1,
            }
        ))
    );
    let remedy = g.unsupported_edges().expect("valid")[0].remedy();
    assert!(remedy.contains("nothing anywhere consumes"), "{remedy}");
}

/// One other task naming the producer's output is enough to remove the
/// producer-side evidence: the output is declared, so this edge is a question
/// about the edge.
#[test]
fn a_producer_someone_else_consumes_leaves_the_cause_undetermined() {
    let g = graph(vec![
        artifact_node("a", &[], &[], &["registry.json"]),
        artifact_node("b", &["a"], &["other.json"], &[]),
        artifact_node("reader", &["a"], &["registry.json"], &[]),
    ]);
    assert_eq!(
        support(&g, "b", "a"),
        Some(EdgeSupport::Unsupported(LikelyCause::Undetermined))
    );
}

/// The ordering-only shape: the dependency produces no artifact and declares
/// only source files, the dependent consumes artifacts. The two ends name
/// resources in different vocabularies, so no overlap was ever possible.
#[test]
fn an_edge_from_an_artifact_consumer_to_a_source_only_producer_is_ordering_only() {
    let g = graph(vec![
        TaskNode {
            writes: vec![path("src/cli.rs"), path("src/cli_tests.rs")],
            ..TaskNode::new("surface", NodeRole::Work)
        },
        artifact_node(
            "command",
            &["surface"],
            &["registry.json"],
            &["report.json"],
        ),
    ]);
    assert_eq!(
        support(&g, "command", "surface"),
        Some(EdgeSupport::OrderingOnly)
    );
    assert!(
        g.unsupported_edges().expect("valid").is_empty(),
        "ordering-only is never a defect"
    );
    let edge = g
        .classify_edges()
        .expect("valid")
        .into_iter()
        .find(|edge| edge.dependency == "surface")
        .expect("classified");
    assert!(
        edge.remedy().contains("leave it alone"),
        "{}",
        edge.remedy()
    );
}

/// The one thing the ordering-only rule must not swallow. A task writing a
/// registry as a plain *file* while its dependent reads the *artifact* of the
/// same name is not speaking a different vocabulary — it is under-declaring,
/// and that is the finding this whole change exists to keep.
#[test]
fn a_source_only_producer_writing_the_path_the_dependent_reads_is_not_ordering_only() {
    let g = graph(vec![
        TaskNode {
            writes: vec![path("src/ingest.rs"), path(".archon/data/registry.json")],
            ..TaskNode::new("ingest", NodeRole::Work)
        },
        artifact_node(
            "coverage",
            &["ingest"],
            &[".archon/data/registry.json"],
            &[],
        ),
    ]);
    assert!(
        matches!(
            support(&g, "coverage", "ingest"),
            Some(EdgeSupport::Unsupported(_))
        ),
        "same path, different declared kind, must stay reported"
    );
}

/// The rule the whole suite rests on. `b` declares no consumption at all, so
/// nothing can be concluded about its edge — not that it is unsupported, not
/// that it is real.
#[test]
fn an_edge_is_never_classified_when_the_dependent_declares_no_consumption() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &["out.json"]),
        node("b", NodeRole::Work, &["a"], &[], &[]),
    ]);
    assert!(g.classify_edges().expect("valid").is_empty());
}

#[test]
fn an_edge_is_never_classified_when_the_dependency_declares_no_production() {
    let g = graph(vec![
        node("a", NodeRole::Work, &[], &[], &[]),
        node("b", NodeRole::Work, &["a"], &["other.json"], &[]),
    ]);
    assert!(g.classify_edges().expect("valid").is_empty());
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
    assert_eq!(support(&g, "wave", "producer"), Some(EdgeSupport::Dataflow));
    assert!(g.unsupported_edges().expect("valid").is_empty());
}

#[test]
fn edge_classification_rejects_a_structurally_invalid_graph_rather_than_guessing() {
    let g = graph(vec![node(
        "b",
        NodeRole::Work,
        &["missing"],
        &["x"],
        &["y"],
    )]);
    assert!(
        g.classify_edges().is_err(),
        "an unknown dependency is an error"
    );
}
