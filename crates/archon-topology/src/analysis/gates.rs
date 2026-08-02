//! Gate dominance — which irreversible nodes can be reached without passing a
//! gate first.

use std::collections::BTreeMap;

use petgraph::algo::dominators::simple_fast;

use crate::error::TopologyError;
use crate::index::GraphIndex;
use crate::ir::{NodeRole, PermissionClass, TaskGraph};

/// Sentinel weight for the synthetic root. `usize::MAX` cannot collide with a
/// real position because a graph with `usize::MAX` nodes is not constructible.
const VIRTUAL_ROOT: usize = usize::MAX;

impl TaskGraph {
    /// Node ids with [`PermissionClass::Irreversible`] that no gate dominates.
    ///
    /// A gate dominates a node when *every* path from the graph's entry to that
    /// node passes through the gate — the standard dominator relation, computed
    /// over a copy of the DAG augmented with a synthetic root joined to every
    /// dependency-free node. Without the synthetic root a multi-root DAG has no
    /// single entry to compute dominators from.
    ///
    /// Milestone 1 evaluates dominance by a gate's *presence*, because a static
    /// graph has no notion of a gate having been *passed*. Milestone 3 narrows
    /// this to gates actually passed in the executed prefix; the graph-shaped
    /// half of that question is answered here and does not change.
    ///
    /// Results are in `TaskGraph::nodes` order. A graph with no irreversible
    /// nodes returns empty — including the common case where the lowering had
    /// no permission information and reported everything `Safe`.
    pub fn ungated_irreversible(&self) -> Result<Vec<String>, TopologyError> {
        let dominating = self.dominating_gates()?;
        Ok(self
            .nodes
            .iter()
            .filter(|node| node.permission == PermissionClass::Irreversible)
            .filter(|node| dominating.get(&node.id).is_none_or(Vec::is_empty))
            .map(|node| node.id.clone())
            .collect())
    }

    /// For every node, the gates that strictly dominate it, nearest first.
    ///
    /// Exposed because the dominance relation is the interesting part and
    /// [`TaskGraph::ungated_irreversible`] only reports its negative half; a
    /// caller explaining *why* an action was permitted needs the gate names.
    pub fn dominating_gates(&self) -> Result<BTreeMap<String, Vec<String>>, TopologyError> {
        let index = GraphIndex::build(self)?;

        let mut augmented = index.graph.clone();
        let root = augmented.add_node(VIRTUAL_ROOT);
        for (position, node) in self.nodes.iter().enumerate() {
            if node.depends_on.is_empty() {
                augmented.add_edge(root, index.node_index[position], ());
            }
        }

        let dominators = simple_fast(&augmented, root);

        let mut result = BTreeMap::new();
        for (position, node) in self.nodes.iter().enumerate() {
            let gates = dominators
                .strict_dominators(index.node_index[position])
                .map(|chain| {
                    chain
                        .map(|index| augmented[index])
                        .filter(|&dominator| dominator != VIRTUAL_ROOT)
                        .filter(|&dominator| self.nodes[dominator].role.is_gate())
                        .map(|dominator| self.nodes[dominator].id.clone())
                        .collect()
                })
                // `None` means unreachable from the synthetic root, which a
                // validated DAG cannot produce. Reporting "no dominating gate"
                // is the conservative reading either way.
                .unwrap_or_default();
            result.insert(node.id.clone(), gates);
        }
        Ok(result)
    }

    /// Node ids whose role is a gate, in graph order.
    #[must_use]
    pub fn gate_nodes(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|node| matches!(node.role, NodeRole::Gate(_)))
            .map(|node| node.id.clone())
            .collect()
    }
}
