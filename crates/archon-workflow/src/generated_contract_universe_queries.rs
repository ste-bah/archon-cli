//! Contract-shape questions the validator asks about a set of canonical tasks.
//!
//! Split from `generated_contract_a.rs` for the 500-line ceiling; these are
//! methods on `ContractTaskUniverse`, which is defined there.

use super::*;

impl ContractTaskUniverse {
    /// How many of these tasks declare a deliverable contract of their own.
    ///
    /// Each such contract needs its own proof, and acceptance is per ITEM — so
    /// an item claiming two of them can retire both on one story.
    pub(super) fn contracted_task_count(&self, task_ids: &[String]) -> usize {
        task_ids
            .iter()
            .filter(|id| self.tasks_with_deliverable_contracts.contains(*id))
            .count()
    }
}
