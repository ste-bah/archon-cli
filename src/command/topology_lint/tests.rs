//! The lints against the two shapes `adversarial-review` had before and after
//! it became a per-task stage, and against a recorded trace.
//!
//! The real seventeen-task PRD corpus has its own file, [`real_corpus`].

use archon_topology::ir::{FanoutSpec, GateKind, NodeRole, TaskGraph, TaskNode, WriteTarget};
use archon_topology::{DiamondFinding, GraphOrigin};

use super::*;

mod real_corpus;

use real_corpus::fixture_root;

// ------------------------------------------- adversarial-review, before/after

fn verifier(id: &str, agent: &str, depends_on: &[&str]) -> TaskNode {
    TaskNode {
        depends_on: depends_on.iter().map(|id| (*id).to_string()).collect(),
        agent: Some(agent.to_string()),
        ..TaskNode::new(id, NodeRole::Verify)
    }
}

fn shaped(nodes: Vec<TaskNode>) -> TaskGraph {
    TaskGraph {
        nodes,
        ..TaskGraph::new(
            "scaffold",
            GraphOrigin::Workflow {
                run_id: "scaffold".to_string(),
            },
        )
    }
}

/// The shape `workflow_live_generated_scaffold.rs` produces today:
/// `implementation-wave` FANOUT → `verification-wave` PARALLEL →
/// `adversarial-review` PARALLEL → `cross-cutting-review` REDUCE.
fn current_scaffold_shape() -> TaskGraph {
    shaped(vec![
        TaskNode {
            fanout: Some(FanoutSpec {
                source: None,
                max_parallelism: None,
            }),
            ..TaskNode::new("implementation-wave", NodeRole::Work)
        },
        verifier("verification-wave", "verifier", &["implementation-wave"]),
        verifier(
            "adversarial-review",
            "adversarial-reviewer",
            &["verification-wave"],
        ),
        TaskNode {
            depends_on: vec!["adversarial-review".to_string()],
            ..TaskNode::new("cross-cutting-review", NodeRole::Reduce)
        },
    ])
}

/// The shape before the change: `adversarial-review` was the single terminal
/// REDUCE over all tasks, so the fan-out's fold had exactly one verification
/// stage feeding it.
fn pre_change_scaffold_shape() -> TaskGraph {
    shaped(vec![
        TaskNode {
            fanout: Some(FanoutSpec {
                source: None,
                max_parallelism: None,
            }),
            ..TaskNode::new("implementation-wave", NodeRole::Work)
        },
        verifier("verification-wave", "verifier", &["implementation-wave"]),
        TaskNode {
            depends_on: vec!["verification-wave".to_string()],
            ..TaskNode::new("adversarial-review", NodeRole::Reduce)
        },
    ])
}

#[test]
fn diamond_conformance_passes_on_the_current_per_task_review_shape() {
    let report = current_scaffold_shape()
        .diamond_conformance()
        .expect("valid graph");
    assert!(
        report.is_clean(),
        "the shipped shape must lint clean: {:?}",
        report.findings
    );
    let score = report
        .diversity
        .iter()
        .find(|score| score.reducer == "cross-cutting-review")
        .expect("the reduce is scored");
    assert_eq!(score.verifiers.len(), 2, "two verification stages feed it");
    assert_eq!(score.distinct_agents, 2, "and they are different reviewers");
}

#[test]
fn diamond_conformance_fails_on_the_pre_change_terminal_reduce_shape() {
    let report = pre_change_scaffold_shape()
        .diamond_conformance()
        .expect("valid graph");
    assert_eq!(
        report.findings,
        vec![DiamondFinding::SoleVerifier {
            reducer: "adversarial-review".to_string(),
            verifier: "verification-wave".to_string(),
        }],
        "one verifier feeding the fold is one reviewer, not a panel"
    );
    assert!(
        report.findings[0].remedy().contains("verification-wave"),
        "the remedy names the stage to change"
    );
}

/// A fan-out folded straight into a reduce with nothing verifying it. This is
/// the check that has nothing to do with diversity: there is no review at all.
#[test]
fn diamond_conformance_flags_a_fanout_that_reduces_without_verification() {
    let graph = shaped(vec![
        TaskNode {
            fanout: Some(FanoutSpec {
                source: None,
                max_parallelism: None,
            }),
            ..TaskNode::new("wave", NodeRole::Work)
        },
        TaskNode {
            depends_on: vec!["wave".to_string()],
            ..TaskNode::new("fold", NodeRole::Reduce)
        },
    ]);
    let report = graph.diamond_conformance().expect("valid graph");
    assert_eq!(
        report.findings,
        vec![DiamondFinding::UnverifiedFanout {
            fanout: "wave".to_string(),
            reducer: "fold".to_string(),
        }]
    );
}

/// A gate between the fan-out and the fold is not verification. It blocks; it
/// does not check. The lint must not accept one as a substitute.
#[test]
fn a_gate_between_fanout_and_reduce_does_not_count_as_verification() {
    let graph = shaped(vec![
        TaskNode {
            fanout: Some(FanoutSpec {
                source: None,
                max_parallelism: None,
            }),
            ..TaskNode::new("wave", NodeRole::Work)
        },
        TaskNode {
            depends_on: vec!["wave".to_string()],
            ..TaskNode::new("checkpoint", NodeRole::Gate(GateKind::Checkpoint))
        },
        TaskNode {
            depends_on: vec!["checkpoint".to_string()],
            ..TaskNode::new("fold", NodeRole::Reduce)
        },
    ]);
    let report = graph.diamond_conformance().expect("valid graph");
    assert!(matches!(
        report.findings.as_slice(),
        [DiamondFinding::UnverifiedFanout { .. }]
    ));
}

// ------------------------------------------------- the recorded-graph source

/// The end-to-end path for stop-rule fusion: `FileRead` and `FileWritten`
/// records land in `trace.jsonl`, reconstruction turns them into `reads` and
/// `writes`, and the lint sees a coupling between two branches nothing ordered.
///
/// This is the loop the design said was blocked. It was blocked on the trace
/// having no read side at all, not on a dataflow contract.
#[test]
fn a_recorded_trace_with_reads_and_writes_reports_the_coupling() {
    use archon_topology::trace::{TopologyPaths, TraceKind, TraceRecord};

    let temp = tempfile::tempdir().expect("temp dir");
    let paths = TopologyPaths::for_project(temp.path());
    let writer = paths.writer("turn-1").expect("writer");
    let shared = WriteTarget::Path("src/shared.rs".to_string());

    for record in [
        TraceRecord::new("2026-08-02T00:00:00Z", "turn-1", TraceKind::AgentSpawned)
            .with_node("branch-a")
            .with_parent("turn"),
        TraceRecord::new("2026-08-02T00:00:01Z", "turn-1", TraceKind::AgentSpawned)
            .with_node("branch-b")
            .with_parent("turn"),
        TraceRecord::new("2026-08-02T00:00:02Z", "turn-1", TraceKind::FileWritten)
            .with_node("branch-a")
            .with_writes(vec![shared.clone()]),
        TraceRecord::new("2026-08-02T00:00:03Z", "turn-1", TraceKind::FileRead)
            .with_node("branch-b")
            .with_reads(vec![shared.clone()]),
    ] {
        writer.append(&record).expect("append");
    }

    let graph = load_graph(temp.path(), &LintSource::Graph("turn-1".to_string()))
        .expect("the recorded graph loads");
    let report = graph.stop_rule_fusion().expect("valid graph");
    let coupled = report
        .coupled
        .iter()
        .find(|pair| pair.reader == "branch-b")
        .expect("branch-b reads what branch-a wrote");
    assert_eq!(coupled.writer, "branch-a");
    assert_eq!(coupled.targets, vec![shared]);
    assert!(coupled.remedy().contains("branch-a"));
}

#[test]
fn a_graph_id_with_neither_a_declared_graph_nor_a_trace_is_an_error() {
    let temp = tempfile::tempdir().expect("temp dir");
    let error = load_graph(temp.path(), &LintSource::Graph("absent".to_string()))
        .expect_err("nothing recorded under that id");
    assert!(error.to_string().contains("absent"), "{error}");
}

// ------------------------------------------------------------- source flags

#[test]
fn lint_requires_exactly_one_source() {
    let error = LintSource::from_flags(None, None, None).expect_err("no source is an error");
    let message = error.to_string();
    assert!(message.contains("--tasks"), "{message}");
    assert!(message.contains("--spec-file"), "{message}");
    assert!(message.contains("--graph"), "{message}");

    let both = LintSource::from_flags(Some(Path::new("a")), None, Some("b"))
        .expect_err("two sources is an error");
    assert!(both.to_string().contains("exactly one"));
}

/// The slash surface parses its own tokens because it never sees clap. An
/// unrecognised flag must be refused rather than skipped: a lint report of
/// something other than what was asked for is worse than no report.
#[test]
fn the_slash_surface_parses_the_same_three_flags_and_refuses_anything_else() {
    let root = fixture_root();
    let args = vec!["--tasks".to_string(), root.display().to_string()];
    let text = crate::command::workflow::lint_from_slash_args(&root, &args)
        .expect("the slash form reaches the same report");
    assert!(text.contains("## dependency edges"));

    let unknown = vec!["--everything".to_string(), "x".to_string()];
    let error = crate::command::workflow::lint_from_slash_args(&root, &unknown)
        .expect_err("an unknown flag is refused");
    assert!(error.to_string().contains("--everything"), "{error}");

    let dangling = vec!["--tasks".to_string()];
    assert!(
        crate::command::workflow::lint_from_slash_args(&root, &dangling).is_err(),
        "a flag with no value is refused rather than treated as absent"
    );
}
