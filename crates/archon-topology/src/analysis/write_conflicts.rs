//! Concurrently-live nodes writing the same target.

use std::collections::BTreeSet;

use crate::error::TopologyError;
use crate::index::GraphIndex;
use crate::ir::{TaskGraph, WriteTarget};

/// Two nodes that write the same target with no dependency path between them,
/// so nothing orders their writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteConflict {
    /// Earlier of the pair in `TaskGraph::nodes` order.
    pub left: String,
    /// Later of the pair in `TaskGraph::nodes` order.
    pub right: String,
    /// Targets both nodes write, sorted for determinism.
    pub targets: Vec<WriteTarget>,
}

impl TaskGraph {
    /// Node pairs with overlapping `writes` and no path between them in either
    /// direction.
    ///
    /// **Silent when `writes` is empty.** An empty `writes` means *unknown*,
    /// not *writes nothing*, so a node with no declared targets is skipped
    /// entirely rather than treated as conflict-free. That makes this analysis
    /// meaningful for `/workflow` immediately — `expected_target_files`
    /// populates it — and for teams only once executors declare write targets.
    ///
    /// Overlap is exact-match on the target string. Glob-versus-glob
    /// intersection is deliberately not attempted here: doing it properly
    /// requires the fail-safe matcher in
    /// `archon_workflow::write_coordinator::write_plan`, which treats a
    /// malformed glob as *conflicting*, and this crate's dependency budget does
    /// not admit `globset`. Milestone 3's single-writer invariant extends that
    /// coordinator rather than reimplementing it, so exact match is the right
    /// conservative floor here: it under-reports, never over-reports.
    pub fn write_conflicts(&self) -> Result<Vec<WriteConflict>, TopologyError> {
        let index = GraphIndex::build(self)?;
        let reachable = index.descendants(self);

        let mut conflicts = Vec::new();
        for (left, left_node) in self.nodes.iter().enumerate() {
            if !left_node.writes_are_known() {
                continue;
            }
            let left_targets: BTreeSet<&WriteTarget> = left_node.writes.iter().collect();

            for (right, right_node) in self.nodes.iter().enumerate().skip(left + 1) {
                if !right_node.writes_are_known() {
                    continue;
                }
                // Ordered by a dependency path in either direction ⇒ the writes
                // are sequenced and cannot race.
                if reachable[left][right] || reachable[right][left] {
                    continue;
                }

                let overlap: Vec<WriteTarget> = right_node
                    .writes
                    .iter()
                    .filter(|target| left_targets.contains(target))
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();

                if !overlap.is_empty() {
                    conflicts.push(WriteConflict {
                        left: left_node.id.clone(),
                        right: right_node.id.clone(),
                        targets: overlap,
                    });
                }
            }
        }
        Ok(conflicts)
    }
}
