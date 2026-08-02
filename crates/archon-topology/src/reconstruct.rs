//! Post-hoc skeleton recovery for turns that never declared a graph.
//!
//! # What this is, and what it is not
//!
//! A `/workflow` run lowers to a [`TaskGraph`] before it executes, so the fold
//! reads an authored graph. An ordinary coding turn declares nothing: it just
//! calls tools and occasionally spawns a subagent. This module builds a graph
//! from what the trace observed after the fact.
//!
//! **It recovers structure, not intent.** A dependency edge here means "the
//! trace says B was spawned by A", which is a containment fact, not a dataflow
//! fact. Two nodes with no edge between them may have been ordered by something
//! the trace never saw. Concretely:
//!
//! - `depends_on` is spawn parentage, so it is a tree, never a diamond, even
//!   when the real work joined.
//! - `consumes` is left empty, and per the crate's unknown-dataflow rule that
//!   means *unknown*. It is not "this node consumed nothing".
//! - `writes` and `reads` are whatever the trace observed, which is a lower
//!   bound in both directions: a tool that touched a file without the tap
//!   noticing contributes nothing.
//!
//! That makes a reconstructed graph adequate for the outcome corpus — span,
//! fan-out width, retry counts, observed conflicts — and adequate for the
//! milestone 4 fusion lint only to the extent the taps observed both halves of
//! a dataflow. It remains inadequate for [`crate::TaskGraph::fake_edges`],
//! which reasons about *declared* intent: an edge this module invented from
//! spawn parentage is not a claim anybody made.
//!
//! The origin is supplied by the caller rather than fixed to
//! [`GraphOrigin::Session`]: a workflow run whose `events.jsonl` is projected
//! after the fact is still a workflow, and forcing it to claim otherwise would
//! break the corpus join on `run_id`.

use std::collections::BTreeMap;

use crate::ir::{GraphOrigin, NodeRole, PermissionClass, TaskGraph, TaskNode, WriteTarget};
use crate::trace::{TraceKind, TraceRecord};

/// Node id used when a record carries no attribution.
///
/// A turn always has a root even if nothing named it: tool calls made directly
/// by the top-level agent belong somewhere, and inventing a root is more honest
/// than dropping them.
pub const ROOT_NODE_ID: &str = "turn";

/// Build a skeleton graph from an observed trace.
///
/// Deterministic: node order follows first appearance in the trace, and every
/// per-node collection is sorted and deduplicated. Two folds of the same trace
/// therefore produce byte-identical graphs, which is what makes the fold
/// idempotent in the interesting sense rather than merely the row-key sense.
#[must_use]
pub fn reconstruct_graph(
    graph_id: &str,
    origin: GraphOrigin,
    records: &[TraceRecord],
) -> TaskGraph {
    let mut graph = TaskGraph::new(graph_id, origin);

    // Insertion-ordered accumulation. `BTreeMap` keeps the per-node state
    // sorted for determinism; `order` preserves first-appearance for the node
    // vector, which reads better and makes the earliest-tie-break in
    // `critical_path` mean "earliest observed".
    let mut nodes: BTreeMap<String, NodeDraft> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();

    for record in records {
        // Unknown kinds are skipped rather than guessed at. The format is
        // additive on purpose and an older reader must not invent semantics for
        // a record it does not understand.
        if record.kind == TraceKind::Unknown || record.kind == TraceKind::GraphDeclared {
            continue;
        }

        let node_id = if record.node_id.is_empty() {
            ROOT_NODE_ID
        } else {
            record.node_id.as_str()
        };
        touch(&mut nodes, &mut order, node_id);

        if let Some(parent) = record.parent_node_id.as_deref()
            && !parent.is_empty()
            && parent != node_id
        {
            touch(&mut nodes, &mut order, parent);
            // Parent edges are recorded on the child, so the graph reads in
            // dependency order rather than spawn order.
            if let Some(draft) = nodes.get_mut(node_id)
                && !draft.depends_on.contains(&parent.to_string())
            {
                draft.depends_on.push(parent.to_string());
            }
        }

        let Some(draft) = nodes.get_mut(node_id) else {
            continue;
        };

        if let Some(agent) = record.agent.as_deref()
            && draft.agent.is_none()
        {
            draft.agent = Some(agent.to_string());
        }

        // Permission is the high-water mark across everything the node did. A
        // node that made one irreversible call is an irreversible node.
        if let Some(permission) = record.permission
            && permission > draft.permission
        {
            draft.permission = permission;
        }

        for target in &record.writes {
            if !draft.writes.contains(target) {
                draft.writes.push(target.clone());
            }
        }

        for target in &record.reads {
            if !draft.reads.contains(target) {
                draft.reads.push(target.clone());
            }
        }

        match record.kind {
            TraceKind::AgentSpawned => draft.spawned = true,
            TraceKind::Verification => draft.verified = true,
            TraceKind::GatePassed => draft.gated = true,
            TraceKind::ToolAttempt => {
                draft.tool_attempts += 1;
                if record.attempt.is_some_and(|attempt| attempt > 0) {
                    draft.retries += 1;
                }
            }
            TraceKind::Retry => draft.retries += 1,
            _ => {}
        }
    }

    for id in order {
        let Some(draft) = nodes.remove(&id) else {
            continue;
        };
        let mut node = TaskNode::new(&id, draft.role());
        node.depends_on = {
            let mut depends_on = draft.depends_on;
            depends_on.sort();
            depends_on.dedup();
            depends_on
        };
        node.writes = {
            let mut writes = draft.writes;
            writes.sort();
            writes.dedup();
            writes
        };
        // Observed reads, which — like `writes` — are a lower bound: a read the
        // tap never saw contributes nothing. Empty therefore stays *unknown*,
        // and the fusion lint declines to conclude anything about such a node.
        node.reads = {
            let mut reads = draft.reads;
            reads.sort();
            reads.dedup();
            reads
        };
        node.permission = draft.permission;
        node.agent = draft.agent;
        // `consumes` stays empty: unknown, not nothing. See the module note.
        graph.nodes.push(node);
    }

    // A reconstructed graph never declares a budget, so the defaults stand
    // except for the two facts the trace does establish.
    graph.budget.max_agents = u32::try_from(graph.nodes.len()).unwrap_or(u32::MAX).max(1);
    graph.budget.max_rounds = 1;
    graph
}

/// Register a node id on first sight, preserving first-appearance order.
fn touch(nodes: &mut BTreeMap<String, NodeDraft>, order: &mut Vec<String>, id: &str) {
    if !nodes.contains_key(id) {
        nodes.insert(id.to_string(), NodeDraft::default());
        order.push(id.to_string());
    }
}

#[derive(Debug, Default)]
struct NodeDraft {
    depends_on: Vec<String>,
    writes: Vec<WriteTarget>,
    reads: Vec<WriteTarget>,
    permission: PermissionClass,
    agent: Option<String>,
    spawned: bool,
    verified: bool,
    gated: bool,
    tool_attempts: usize,
    retries: usize,
}

impl NodeDraft {
    /// Infer a role from observed behaviour.
    ///
    /// This is the weakest inference in the module and is deliberately coarse.
    /// A gate observed passing is a gate; a node that only ever verified is a
    /// verifier; a spawned subagent is work; a node that only ever called tools
    /// is a tool node. Anything else is work, which is the role that asserts
    /// least.
    fn role(&self) -> NodeRole {
        if self.gated {
            // `Checkpoint` rather than `Human`: the trace saw a gate pass, and
            // nothing in it distinguishes a human approval from a recorded
            // resumption point. `Checkpoint` is the weaker claim.
            return NodeRole::Gate(crate::ir::GateKind::Checkpoint);
        }
        if self.verified && !self.spawned {
            return NodeRole::Verify;
        }
        if self.spawned {
            return NodeRole::Work;
        }
        if self.tool_attempts > 0 {
            return NodeRole::Tool;
        }
        NodeRole::Work
    }
}

/// Retry counts per node, as observed. Kept separate from the graph because the
/// IR is a static description of shape and retries are a runtime fact.
#[must_use]
pub fn observed_retries(records: &[TraceRecord]) -> BTreeMap<String, usize> {
    let mut retries: BTreeMap<String, usize> = BTreeMap::new();
    for record in records {
        let node_id = if record.node_id.is_empty() {
            ROOT_NODE_ID
        } else {
            record.node_id.as_str()
        };
        let is_retry = record.kind == TraceKind::Retry
            || (record.kind == TraceKind::ToolAttempt
                && record.attempt.is_some_and(|attempt| attempt > 0));
        if is_retry {
            *retries.entry(node_id.to_string()).or_default() += 1;
        }
    }
    retries
}

#[cfg(test)]
mod tests;
