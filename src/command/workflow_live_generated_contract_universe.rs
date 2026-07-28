use std::collections::{BTreeMap, BTreeSet};

use super::super::workflow_live_task_universe::WorkflowV2TaskUniverse;

#[derive(Debug, Clone, Default)]
pub(super) struct ContractTaskUniverse {
    pub(super) canonical: BTreeSet<String>,
    aliases: BTreeMap<String, String>,
    dependencies: BTreeMap<String, Vec<String>>,
}

impl ContractTaskUniverse {
    pub(super) fn from_authoritative(task_universe: Option<&WorkflowV2TaskUniverse>) -> Self {
        let mut out = Self::default();
        let Some(task_universe) = task_universe else {
            return out;
        };
        for task in &task_universe.tasks {
            out.add_canonical(&task.canonical_task_id);
            for alias in &task.aliases {
                out.aliases
                    .insert(alias.trim().to_string(), task.canonical_task_id.clone());
            }
            out.dependencies.insert(
                task.canonical_task_id.clone(),
                super::sorted_unique(task.dependency_ids.clone()),
            );
        }
        out
    }

    fn add_canonical(&mut self, task_id: &str) {
        let canonical = task_id.trim();
        if canonical.is_empty() {
            return;
        }
        self.canonical.insert(canonical.to_string());
        self.aliases
            .insert(canonical.to_string(), canonical.to_string());
        if let Some(short) = super::short_task_alias(canonical) {
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
        self.aliases.get(trimmed).cloned()
    }

    pub(super) fn dependencies_for(&self, task_id: &str) -> Vec<String> {
        self.dependencies.get(task_id).cloned().unwrap_or_default()
    }
}
