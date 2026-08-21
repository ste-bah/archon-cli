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

    pub(super) fn has_deliverable_contract(&self, task_ids: &[String]) -> bool {
        task_ids
            .iter()
            .any(|id| self.tasks_with_deliverable_contracts.contains(id))
    }

    /// Whether any of these tasks declares a command that must be executed.
    pub(super) fn requires_execution(&self, task_ids: &[String]) -> bool {
        task_ids
            .iter()
            .any(|id| self.tasks_requiring_execution.contains(id))
    }

    pub(super) fn add_canonical(&mut self, task_id: &str) {
        let canonical = task_id.trim();
        if canonical.is_empty() {
            return;
        }
        self.canonical.insert(canonical.to_string());
        self.aliases
            .insert(canonical.to_string(), canonical.to_string());
        if let Some(short) = short_task_alias(canonical) {
            self.aliases.insert(short, canonical.to_string());
        }
    }

    pub(super) fn resolve(&self, value: &str) -> Option<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        if self.canonical.is_empty() {
            return Some(trimmed.to_string());
        }
        let mut matches = self
            .aliases
            .iter()
            .filter(|(alias, _)| alias.eq_ignore_ascii_case(trimmed))
            .map(|(_, canonical)| canonical.clone())
            .collect::<BTreeSet<_>>();
        for canonical in &self.canonical {
            let Some((_, suffix)) = canonical.split_once('-') else {
                continue;
            };
            if suffix.contains('-') && suffix.eq_ignore_ascii_case(trimmed) {
                matches.insert(canonical.clone());
            }
        }
        (matches.len() == 1)
            .then(|| matches.into_iter().next())
            .flatten()
    }

    pub(super) fn dependencies_for(&self, task_id: &str) -> Vec<String> {
        self.dependencies.get(task_id).cloned().unwrap_or_default()
    }
}
