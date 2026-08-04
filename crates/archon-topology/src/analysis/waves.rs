//! Topological levels — the replacement for `orchestrator/dag.rs`.

use crate::error::TopologyError;
use crate::index::GraphIndex;
use crate::ir::TaskGraph;

impl TaskGraph {
    /// Group nodes into execution waves. Everything in a wave may run
    /// concurrently; wave `n` may not start before wave `n-1` completes.
    ///
    /// Wave membership is by *depth*, not by petgraph's topological order: a
    /// node sits one past its deepest dependency. Within a wave, nodes keep
    /// their `TaskGraph::nodes` order, which for the team lowering is the order
    /// the decomposition produced them in.
    ///
    /// This reproduces the semantics of the deleted
    /// `archon_core::orchestrator::dag::build_dag_waves` exactly, including
    /// erroring on an unknown dependency id before checking for cycles.
    pub fn waves(&self) -> Result<Vec<Vec<String>>, TopologyError> {
        let index = GraphIndex::build(self)?;
        let depths = index.depths(self);

        let Some(&max_depth) = depths.iter().max() else {
            return Ok(Vec::new());
        };

        let mut waves: Vec<Vec<String>> = vec![Vec::new(); max_depth + 1];
        for (position, node) in self.nodes.iter().enumerate() {
            waves[depths[position]].push(node.id.clone());
        }
        // Depths over a validated DAG are contiguous, so this is defensive
        // only — kept because the old implementation had it and dropping it
        // would be an unforced behaviour difference.
        waves.retain(|wave| !wave.is_empty());
        Ok(waves)
    }

    /// Wave number per node, in `TaskGraph::nodes` order.
    ///
    /// The same computation [`TaskGraph::waves`] performs, exposed for callers
    /// that need to ask "which wave is this node in" without scanning.
    pub fn wave_depths(&self) -> Result<Vec<usize>, TopologyError> {
        let index = GraphIndex::build(self)?;
        Ok(index.depths(self))
    }

    /// Validate structure without computing anything: duplicate ids, unknown
    /// dependencies, cycles.
    pub fn validate(&self) -> Result<(), TopologyError> {
        GraphIndex::build(self).map(|_| ())
    }
}
