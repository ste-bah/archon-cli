//! The one `learning_events` row a fold writes into the shared learning store.
//!
//! Owns the summary row and the shape signals it carries: the event type
//! constant, the confidence model, and the two graph-shape statistics
//! (`fan_out_widths`, `verifier_independence`) that exist only to be reported
//! here. Exactly one row per graph, keyed so a replay upserts.

use anyhow::{Context, Result};
use archon_topology::ir::{NodeRole, TaskGraph};
use cozo::DbInstance;
use std::collections::BTreeMap;

use super::derive::FoldedGraph;
use super::labels::origin_label;

/// `event_type` written into `learning_events` for the per-graph summary.
///
/// **Not** the design's `"topology_outcome"`. `learning_events.event_type` is
/// not a free string: it is written from
/// `archon_learning::models::LearningEventType::as_str`, a closed enum whose
/// twenty-three existing variants are all PascalCase, and both
/// `learning_events:by_type_created_at` index queries in the tree match on that
/// spelling. A snake_case value would parse back as `None` from
/// `LearningEventType::from_str` and would sort oddly next to its siblings. The
/// variant is `TopologyOutcome`; the wire form is this constant.
pub(crate) const TOPOLOGY_OUTCOME_EVENT_TYPE: &str = "TopologyOutcome";

/// Write **one** summary row per graph into the shared `learning_events`
/// relation.
///
/// This is the deliberate exception to "bulk rows to their own file": it makes
/// topology visible to every existing `learning_events` consumer without any of
/// them changing. It is one write per graph and must never become one per node
/// — the whole point of the batched fold is that the shared store sees O(1)
/// writers.
///
/// The row id is derived from the graph id, so a repeat fold upserts the same
/// row rather than adding a second one.
pub(super) fn write_learning_summary(
    db: &DbInstance,
    graph_id: &str,
    workspace_id: &str,
    folded: &FoldedGraph,
) -> Result<()> {
    archon_learning::schema::ensure_learning_schema(db)
        .context("ensure learning schema for topology summary")?;
    debug_assert_eq!(
        archon_learning::models::LearningEventType::TopologyOutcome.as_str(),
        TOPOLOGY_OUTCOME_EVENT_TYPE,
        "the documented wire form and the enum must not drift apart"
    );

    let signal = serde_json::json!({
        "graph_id": graph_id,
        "task_hash": folded.task_hash,
        "origin": origin_label(&folded.graph.origin),
        "node_count": folded.node_count,
        "span": folded.span,
        "work": folded.work,
        "max_parallelism_used": folded.max_parallelism_used,
        "budget_max_parallelism": folded.graph.budget.max_parallelism,
        "wave_widths": folded
            .graph
            .parallelism_profile()
            .map(|profile| profile.wave_widths)
            .unwrap_or_default(),
        "fan_out_widths": fan_out_widths(&folded.graph),
        "verifier_count": folded
            .graph
            .nodes
            .iter()
            .filter(|node| node.role == NodeRole::Verify)
            .count(),
        "verifier_independence": verifier_independence(&folded.graph),
        "gate_nodes": folded.graph.gate_nodes(),
        "ungated_irreversible": folded.graph.ungated_irreversible().unwrap_or_default(),
        "write_conflicts": folded.write_conflicts,
        "retries_total": folded.retries_total,
        "nodes_failed": folded.nodes_failed,
        "verified": folded.verified,
        "failure_class": folded.failure_class,
        "reconstructed": folded.reconstructed,
        "truncated_trace": folded.truncated_trace,
    });

    let event = archon_learning::models::LearningEvent {
        // Deterministic in the graph id: the idempotence of the whole fold
        // rests on this being an upsert key rather than a fresh uuid.
        event_id: format!("topology-outcome-{graph_id}"),
        workspace_id: workspace_id.to_string(),
        event_type: archon_learning::models::LearningEventType::TopologyOutcome,
        source_artifact_id: graph_id.to_string(),
        outcome_artifact_id: None,
        signal,
        // Confidence maps from verification strength: a graph with independent
        // verifiers and no failures is worth more than a bare reconstruction.
        confidence: confidence_from_verification(folded),
        provenance_record_id: String::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    archon_learning::store::insert_learning_event(db, &event)
        .context("insert topology_outcome learning event")
}

/// Confidence in the range `[0.1, 0.95]`.
///
/// A reconstructed skeleton is a weaker observation than a declared graph, and
/// a graph with independent verifiers that all passed is a stronger one.
fn confidence_from_verification(folded: &FoldedGraph) -> f32 {
    let mut confidence: f32 = if folded.reconstructed { 0.3 } else { 0.6 };
    if folded.verified {
        confidence += 0.2;
    }
    if folded.nodes_failed > 0 {
        confidence -= 0.15;
    }
    if folded.truncated_trace {
        confidence -= 0.1;
    }
    confidence.clamp(0.1, 0.95)
}

/// Widths of every fan-out in the graph: for each node, how many nodes depend
/// directly on it, wherever that is more than one.
pub(super) fn fan_out_widths(graph: &TaskGraph) -> Vec<usize> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for node in &graph.nodes {
        for dependency in &node.depends_on {
            *counts.entry(dependency.as_str()).or_default() += 1;
        }
    }
    let mut widths: Vec<usize> = counts.into_values().filter(|width| *width > 1).collect();
    widths.sort_unstable();
    widths
}

/// How many verifiers share no dependency with each other.
///
/// A crude proxy, and named as one. Three verifiers all fed by the same
/// producer are not three independent checks; three verifiers with disjoint
/// dependency sets plausibly are. Milestone 4 replaces this with something that
/// reasons about declared dataflow, which does not exist yet.
pub(super) fn verifier_independence(graph: &TaskGraph) -> usize {
    let verifiers: Vec<&archon_topology::ir::TaskNode> = graph
        .nodes
        .iter()
        .filter(|node| node.role == NodeRole::Verify)
        .collect();
    let mut independent = 0;
    for (index, left) in verifiers.iter().enumerate() {
        let disjoint = verifiers.iter().enumerate().all(|(other, right)| {
            other == index
                || !left
                    .depends_on
                    .iter()
                    .any(|dependency| right.depends_on.contains(dependency))
        });
        if disjoint {
            independent += 1;
        }
    }
    independent
}
