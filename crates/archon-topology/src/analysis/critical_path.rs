//! Longest dependency chain — the graph's span.

use crate::error::TopologyError;
use crate::index::GraphIndex;
use crate::ir::TaskGraph;

/// The longest chain of dependent nodes in the graph.
///
/// Unweighted: milestone 1 has no durations, so every node costs 1 and the span
/// is a node count. Milestone 2 folds observed durations into the trace, at
/// which point a weighted variant becomes meaningful.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CriticalPath {
    /// Node ids from a root to the deepest sink, in execution order.
    pub nodes: Vec<String>,
}

impl CriticalPath {
    /// Number of waves the path forces, i.e. the graph's span. Zero for an
    /// empty graph.
    #[must_use]
    pub fn span(&self) -> usize {
        self.nodes.len()
    }
}

impl TaskGraph {
    /// The longest chain of dependent nodes.
    ///
    /// Ties are broken toward the earliest node in `TaskGraph::nodes` order so
    /// the result is deterministic across runs — a graph whose critical path
    /// flickered between equal-length chains would produce unstable outcome
    /// rows in the milestone 2 corpus.
    pub fn critical_path(&self) -> Result<CriticalPath, TopologyError> {
        let index = GraphIndex::build(self)?;
        let depths = index.depths(self);

        // Deepest node, earliest-first on ties.
        let Some(deepest) = (0..self.nodes.len())
            .max_by_key(|&position| (depths[position], std::cmp::Reverse(position)))
        else {
            return Ok(CriticalPath { nodes: Vec::new() });
        };

        let mut chain = vec![deepest];
        let mut cursor = deepest;
        while depths[cursor] > 0 {
            let target_depth = depths[cursor] - 1;
            let Some(previous) = self.nodes[cursor]
                .depends_on
                .iter()
                .filter_map(|dependency| index.by_id.get(dependency).copied())
                .filter(|&candidate| depths[candidate] == target_depth)
                .min()
            else {
                // Unreachable on a validated graph: a node at depth d has, by
                // construction, at least one dependency at depth d-1.
                break;
            };
            chain.push(previous);
            cursor = previous;
        }
        chain.reverse();

        Ok(CriticalPath {
            nodes: chain
                .into_iter()
                .map(|position| self.nodes[position].id.clone())
                .collect(),
        })
    }
}
