//! Turning a graph plus its trace into the numbers the fold stores.
//!
//! Owns [`FoldedGraph`] — everything one fold derived, before any of it is
//! written — and the pure functions that produce it. No I/O and no database, so
//! the derivation can be asserted on directly and the write stays a single
//! obvious transaction.

use std::collections::BTreeMap;

use archon_topology::ir::{GraphOrigin, NodeRole, TaskGraph};
use archon_topology::reconstruct::{observed_retries, reconstruct_graph};
use archon_topology::trace::{TraceKind, TraceReadout, TraceRecord};

/// Everything the fold derived from one graph's trace, before it is written.
#[derive(Debug, Clone)]
pub(crate) struct FoldedGraph {
    pub graph: TaskGraph,
    pub task_hash: String,
    pub node_count: usize,
    pub span: usize,
    pub work: usize,
    pub max_parallelism_used: usize,
    pub retries: BTreeMap<String, usize>,
    pub outcomes: BTreeMap<String, String>,
    pub durations: BTreeMap<String, u64>,
    pub nodes_failed: usize,
    pub retries_total: usize,
    pub write_conflicts: usize,
    pub verified: bool,
    pub failure_class: String,
    pub truncated_trace: bool,
    pub reconstructed: bool,
}

/// Derive metrics from a graph and its trace. Pure; no I/O.
///
/// `fallback_origin` labels a reconstruction. It is ignored when `declared` is
/// present, because a declared graph already knows what it is.
pub(crate) fn derive(
    graph_id: &str,
    fallback_origin: GraphOrigin,
    declared: Option<TaskGraph>,
    readout: &TraceReadout,
    goal_text: &str,
) -> FoldedGraph {
    let reconstructed = declared.is_none();
    let graph =
        declared.unwrap_or_else(|| reconstruct_graph(graph_id, fallback_origin, &readout.records));

    // The analyses are total on an acyclic graph and fallible otherwise. A
    // malformed graph must not strand its trace, so a failure degrades the
    // metric rather than the fold: span falls back to node count, which is the
    // correct upper bound, and occupancy to 0, which claims nothing.
    let span = graph
        .critical_path()
        .map(|path| path.span())
        .unwrap_or(graph.len());
    let max_parallelism_used = graph
        .parallelism_profile()
        .map(|profile| profile.peak_width)
        .unwrap_or(0);
    let write_conflicts = graph.write_conflicts().map(|c| c.len()).unwrap_or(0);

    let retries = observed_retries(&readout.records);
    let outcomes = node_outcomes(&readout.records);
    let durations = node_durations(&readout.records);

    let nodes_failed = outcomes.values().filter(|state| *state == "failed").count();
    let retries_total = retries.values().sum();
    let verified = graph.nodes.iter().any(|node| node.role == NodeRole::Verify)
        && nodes_failed == 0
        && !readout.records.is_empty();

    FoldedGraph {
        task_hash: archon_topology::task_hash(goal_text),
        node_count: graph.len(),
        span,
        work: graph.len(),
        max_parallelism_used,
        nodes_failed,
        retries_total,
        write_conflicts,
        verified,
        failure_class: failure_class(nodes_failed, readout),
        truncated_trace: readout.truncated_tail,
        reconstructed,
        retries,
        outcomes,
        durations,
        graph,
    }
}

/// Terminal state per node, from the trace.
pub(super) fn node_outcomes(records: &[TraceRecord]) -> BTreeMap<String, String> {
    let mut outcomes: BTreeMap<String, String> = BTreeMap::new();
    for record in records {
        if record.node_id.is_empty() || record.kind == TraceKind::Unknown {
            continue;
        }
        let state = match record.kind {
            TraceKind::NodeFinished if record.error => "failed",
            TraceKind::NodeFinished => "completed",
            TraceKind::NodeStarted | TraceKind::AgentSpawned => "started",
            TraceKind::ToolAttempt if record.blocked => "blocked",
            TraceKind::ToolAttempt if record.error => "errored",
            _ => continue,
        };
        // A terminal state is sticky: a later `started` from a retry must not
        // erase an earlier failure.
        let existing = outcomes.get(&record.node_id).map(String::as_str);
        if matches!(existing, Some("failed")) {
            continue;
        }
        if matches!(existing, Some("completed")) && state != "failed" {
            continue;
        }
        outcomes.insert(record.node_id.clone(), state.to_string());
    }
    outcomes
}

/// Longest observed duration per node.
fn node_durations(records: &[TraceRecord]) -> BTreeMap<String, u64> {
    let mut durations: BTreeMap<String, u64> = BTreeMap::new();
    for record in records {
        let (Some(duration), false) = (record.duration_ms, record.node_id.is_empty()) else {
            continue;
        };
        let entry = durations.entry(record.node_id.clone()).or_default();
        *entry = (*entry).max(duration);
    }
    durations
}

/// Sum of observed node durations, or `-1` when nothing reported one.
///
/// Not wall clock in the strict sense — the trace carries no start and end
/// stamp for the graph as a whole — and reported as a sum rather than
/// pretending otherwise.
pub(super) fn wall_clock_ms(folded: &FoldedGraph) -> usize {
    if folded.durations.is_empty() {
        return 0;
    }
    folded
        .durations
        .values()
        .copied()
        .map(|ms| usize::try_from(ms).unwrap_or(usize::MAX))
        .sum()
}

fn failure_class(nodes_failed: usize, readout: &TraceReadout) -> String {
    if nodes_failed > 0 {
        return "node_failure".to_string();
    }
    if readout.records.iter().any(|record| record.blocked) {
        return "admission_blocked".to_string();
    }
    if readout.truncated_tail {
        return "trace_truncated".to_string();
    }
    String::new()
}
