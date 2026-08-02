//! Petgraph projection of a [`TaskGraph`], shared by every analysis.
//!
//! Built once per analysis call rather than cached on the IR: the IR is a plain
//! serializable value and must stay one, and the graphs involved are small
//! enough (tens of nodes) that rebuilding costs nothing measurable.

use std::collections::HashMap;

use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};

use crate::error::TopologyError;
use crate::ir::TaskGraph;

/// A `TaskGraph` projected onto petgraph, with the id↔position bookkeeping
/// every analysis would otherwise repeat.
///
/// Node weights are positions into [`TaskGraph::nodes`], so an analysis can go
/// from a `NodeIndex` back to the full [`crate::TaskNode`] without a lookup by
/// string.
pub(crate) struct GraphIndex {
    pub(crate) graph: DiGraph<usize, ()>,
    /// `node_index[i]` is the petgraph node for `TaskGraph::nodes[i]`.
    pub(crate) node_index: Vec<NodeIndex>,
    /// Node id → position in `TaskGraph::nodes`.
    pub(crate) by_id: HashMap<String, usize>,
    /// Positions in topological order. Edges run dependency → dependent, so a
    /// node's dependencies always appear before it.
    pub(crate) topo_order: Vec<usize>,
}

impl GraphIndex {
    /// Project `graph`, validating structure.
    ///
    /// Errors, in the order they are detected: duplicate ids, unknown
    /// dependencies, then cycles. Dependency validation precedes the cycle
    /// check so a typo is reported as a typo rather than as a missing edge.
    pub(crate) fn build(graph: &TaskGraph) -> Result<Self, TopologyError> {
        let mut petgraph: DiGraph<usize, ()> = DiGraph::new();
        let mut node_index = Vec::with_capacity(graph.nodes.len());
        let mut by_id: HashMap<String, usize> = HashMap::with_capacity(graph.nodes.len());

        for (position, node) in graph.nodes.iter().enumerate() {
            if by_id.insert(node.id.clone(), position).is_some() {
                return Err(TopologyError::DuplicateNode {
                    id: node.id.clone(),
                });
            }
            node_index.push(petgraph.add_node(position));
        }

        for (position, node) in graph.nodes.iter().enumerate() {
            let to = node_index[position];
            for dependency in &node.depends_on {
                let from_position =
                    by_id
                        .get(dependency)
                        .ok_or_else(|| TopologyError::UnknownDependency {
                            node: node.id.clone(),
                            dependency: dependency.clone(),
                        })?;
                petgraph.add_edge(node_index[*from_position], to, ());
            }
        }

        let topo_order = toposort(&petgraph, None)
            .map_err(|_| TopologyError::Cycle)?
            .into_iter()
            .map(|index| petgraph[index])
            .collect();

        Ok(Self {
            graph: petgraph,
            node_index,
            by_id,
            topo_order,
        })
    }

    /// Wave number per node position: 0 for a node with no dependencies,
    /// otherwise one past the deepest dependency.
    ///
    /// Computed in topological order rather than by the memoized recursion the
    /// old `build_dag_waves` used, which would have recursed forever on a cycle
    /// had the toposort not run first.
    pub(crate) fn depths(&self, graph: &TaskGraph) -> Vec<usize> {
        let mut depths = vec![0usize; graph.nodes.len()];
        for &position in &self.topo_order {
            let depth = graph.nodes[position]
                .depends_on
                .iter()
                .filter_map(|dependency| self.by_id.get(dependency))
                .map(|&dependency_position| depths[dependency_position] + 1)
                .max()
                .unwrap_or(0);
            depths[position] = depth;
        }
        depths
    }

    /// Positions reachable from each node, following dependency → dependent.
    ///
    /// Accumulated in reverse topological order so each node unions sets that
    /// are already complete — one pass, no fixpoint.
    pub(crate) fn descendants(&self, graph: &TaskGraph) -> Vec<Vec<bool>> {
        let count = graph.nodes.len();
        let mut reachable = vec![vec![false; count]; count];
        for &position in self.topo_order.iter().rev() {
            let successors: Vec<usize> = self
                .graph
                .neighbors_directed(self.node_index[position], petgraph::Direction::Outgoing)
                .map(|index| self.graph[index])
                .collect();
            for successor in successors {
                // Cloning the successor's row sidesteps holding two mutable
                // borrows of `reachable` at once. Rows are `count` bools on
                // graphs of a few dozen nodes, so this is not worth a
                // `split_at_mut` dance.
                let successor_row = reachable[successor].clone();
                reachable[position][successor] = true;
                for (target, &is_reachable) in successor_row.iter().enumerate() {
                    if is_reachable {
                        reachable[position][target] = true;
                    }
                }
            }
        }
        reachable
    }
}
