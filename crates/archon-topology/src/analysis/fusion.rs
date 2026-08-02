//! Stop-rule fusion: where the graph's parallel/sequential split disagrees with
//! its dataflow.
//!
//! Two symmetrical mistakes, and this reports both.
//!
//! **Coupling** — two nodes the graph permits to run at the same time, where one
//! reads a target the other writes. Nothing orders them, so the reader sees the
//! file before or after the write depending on scheduling. That work is not
//! parallel; it only looks parallel.
//!
//! **Slack** — two nodes the graph orders, where the downstream one consumes
//! nothing the upstream one produces. The barrier between them buys nothing.
//! When they also have the same role and agent they are one stage split in two
//! ([`FusionKind::Fuse`]); otherwise they are two stages that could run at once
//! ([`FusionKind::Parallelise`]).
//!
//! # Relation to `fake_edges`
//!
//! [`TaskGraph::fake_edges`](crate::FakeEdge) answers "is this *edge*
//! justified", over every edge in the graph. This answers "is this *chain*
//! shaped right", and only fires on a degenerate chain — sole predecessor, sole
//! successor — where the remedy is mechanical. Both can fire on one pair. They
//! are kept separate because the remedies differ: one deletes an edge, the
//! other merges or re-levels two stages.
//!
//! # Silence is the default
//!
//! Coupling needs the reader's `reads` and the writer's `writes`; slack needs
//! the downstream `reads`/`consumes` and the upstream `writes`. Empty means
//! unknown, so a node that declared nothing produces no findings in either
//! direction.

use std::collections::BTreeSet;

use crate::error::TopologyError;
use crate::index::GraphIndex;
use crate::ir::{TaskGraph, WriteTarget};

/// Two concurrent nodes joined by an undeclared read-after-write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoupledPair {
    /// The node that reads.
    pub reader: String,
    /// The node that writes what the reader reads.
    pub writer: String,
    /// The targets both touch, sorted.
    pub targets: Vec<WriteTarget>,
    /// The nearest fan-out both descend from, when there is one. `Some` means
    /// these are two branches of one fan-out and the fan-out itself is the
    /// thing to change; `None` means they are unrelated stages the graph simply
    /// never ordered.
    pub fanout: Option<String>,
}

impl CoupledPair {
    #[must_use]
    pub fn remedy(&self) -> String {
        match &self.fanout {
            Some(fanout) => format!(
                "'{reader}' reads what sibling branch '{writer}' writes, so fan-out '{fanout}' is \
                 not parallel work: split the shared target out of the fan-out, or order the two \
                 branches explicitly",
                reader = self.reader,
                writer = self.writer
            ),
            None => format!(
                "add a dependency from '{reader}' onto '{writer}', or stop '{reader}' reading \
                 what '{writer}' writes — nothing currently orders them",
                reader = self.reader,
                writer = self.writer
            ),
        }
    }
}

/// What to do with a chain that carries no dataflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FusionKind {
    /// Same role and same agent: one stage split in two for no reason.
    Fuse,
    /// Different role or agent: two stages that could run concurrently.
    Parallelise,
}

/// A sequential pair the dataflow does not justify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FusibleChain {
    pub upstream: String,
    pub downstream: String,
    pub kind: FusionKind,
}

impl FusibleChain {
    #[must_use]
    pub fn remedy(&self) -> String {
        match self.kind {
            FusionKind::Fuse => format!(
                "'{up}' and '{down}' have the same role and agent and pass no data between them: \
                 merge them into one stage",
                up = self.upstream,
                down = self.downstream
            ),
            FusionKind::Parallelise => format!(
                "'{down}' consumes nothing '{up}' produces and neither has other neighbours: drop \
                 the edge so they run in the same wave",
                up = self.upstream,
                down = self.downstream
            ),
        }
    }
}

/// Everything [`TaskGraph::stop_rule_fusion`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FusionReport {
    /// Concurrent nodes that are actually coupled. Should be sequential.
    pub coupled: Vec<CoupledPair>,
    /// Sequential nodes that are not actually coupled. Could be one stage, or
    /// could be concurrent.
    pub fusible: Vec<FusibleChain>,
}

impl FusionReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.coupled.is_empty() && self.fusible.is_empty()
    }
}

impl TaskGraph {
    /// Where the graph's ordering disagrees with its dataflow, in both
    /// directions.
    ///
    /// Coupling is reported for every mutually-unordered pair, with the nearest
    /// common fan-out ancestor named when one exists. Restricting the check to
    /// fan-out branches would miss the same defect between two hand-declared
    /// concurrent stages, which is the same bug with a different author.
    ///
    /// A node is never reported as coupled with itself, and a pair is reported
    /// once per direction only when the read-write relation actually holds in
    /// that direction — two nodes that each read what the other writes yield
    /// two findings, because they are two distinct races.
    pub fn stop_rule_fusion(&self) -> Result<FusionReport, TopologyError> {
        let index = GraphIndex::build(self)?;
        let reachable = index.descendants(self);
        let mut report = FusionReport::default();

        for (reader, reader_node) in self.nodes.iter().enumerate() {
            if !reader_node.reads_are_known() {
                continue;
            }
            let reads: BTreeSet<&WriteTarget> = reader_node.reads.iter().collect();

            for (writer, writer_node) in self.nodes.iter().enumerate() {
                if writer == reader || !writer_node.writes_are_known() {
                    continue;
                }
                // Ordered in either direction ⇒ the graph already sequenced
                // them and there is no race to report.
                if reachable[reader][writer] || reachable[writer][reader] {
                    continue;
                }
                let targets: Vec<WriteTarget> = writer_node
                    .writes
                    .iter()
                    .filter(|target| reads.contains(*target))
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                if targets.is_empty() {
                    continue;
                }
                report.coupled.push(CoupledPair {
                    reader: reader_node.id.clone(),
                    writer: writer_node.id.clone(),
                    targets,
                    fanout: nearest_common_fanout(self, &reachable, reader, writer),
                });
            }
        }

        report.fusible = self.fusible_chains(&index);
        Ok(report)
    }

    /// Degenerate chains: sole predecessor, sole successor, no dataflow.
    fn fusible_chains(&self, index: &GraphIndex) -> Vec<FusibleChain> {
        let mut chains = Vec::new();
        for downstream in &self.nodes {
            let [upstream_id] = downstream.depends_on.as_slice() else {
                continue;
            };
            let Some(upstream) = self.node(upstream_id) else {
                continue;
            };
            if dependents(index, &upstream.id) != 1 {
                continue;
            }
            if !upstream.writes_are_known() || !downstream.consumption_is_known() {
                continue;
            }
            if downstream
                .consumes
                .iter()
                .any(|reference| reference.producer == upstream.id)
            {
                continue;
            }
            let reads: BTreeSet<&WriteTarget> = downstream.reads.iter().collect();
            if upstream.writes.iter().any(|target| reads.contains(target)) {
                continue;
            }

            let same_shape = upstream.role == downstream.role && upstream.agent == downstream.agent;
            chains.push(FusibleChain {
                upstream: upstream.id.clone(),
                downstream: downstream.id.clone(),
                kind: if same_shape {
                    FusionKind::Fuse
                } else {
                    FusionKind::Parallelise
                },
            });
        }
        chains
    }
}

fn dependents(index: &GraphIndex, id: &str) -> usize {
    let Some(&position) = index.by_id.get(id) else {
        return 0;
    };
    index
        .graph
        .neighbors_directed(index.node_index[position], petgraph::Direction::Outgoing)
        .count()
}

/// The fan-out node closest to both `left` and `right` that reaches each of
/// them, if any.
///
/// "Closest" is the candidate the most other candidates reach — on a DAG that
/// is the deepest common fan-out ancestor. Node-vector order breaks a tie, so
/// the answer is deterministic; when several fan-outs are genuinely equally
/// deep, naming any of them points at the same defect.
fn nearest_common_fanout(
    graph: &TaskGraph,
    reachable: &[Vec<bool>],
    left: usize,
    right: usize,
) -> Option<String> {
    let candidates: Vec<usize> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(position, node)| {
            node.fanout.is_some() && reachable[*position][left] && reachable[*position][right]
        })
        .map(|(position, _)| position)
        .collect();

    candidates
        .iter()
        .copied()
        .max_by_key(|&candidate| {
            candidates
                .iter()
                .filter(|&&other| other != candidate && reachable[other][candidate])
                .count()
        })
        .map(|position| graph.nodes[position].id.clone())
}
