//! Deliverable admission for [`WorkflowV2ProjectArtifactContext`].
//!
//! Split from `project_artifacts.rs` to hold the 500-line ceiling. These are
//! the methods that read the task universe: which exact deliverables an
//! artifact-only item may write, and which of them the task declared as
//! directories.

use super::project_artifacts::WorkflowV2ProjectArtifactContext;

impl WorkflowV2ProjectArtifactContext {
    /// Admit the exact deliverables an artifact-only item is entitled to write.
    ///
    /// Only for items with no repository `target_files`, and only paths the
    /// host parsed from the task files for that item's canonical tasks — so an
    /// agent cannot widen its own rights and code tasks are untouched. See
    /// `project_artifact_contract_roots` for the rule.
    pub fn add_contract_artifact_paths(
        &mut self,
        universe: &crate::task_universe::WorkflowV2TaskUniverse,
        item: &serde_json::Value,
    ) {
        for path in
            super::project_artifact_contract_roots::contract_artifact_paths_for_item(universe, item)
        {
            if !self.artifact_paths.contains(&path) {
                self.artifact_paths.push(path);
            }
        }
        self.add_directory_artifacts(universe, item);
    }

    /// Record which of this item's declared deliverables the task wrote as
    /// directories, read from the universe where the separator still exists.
    pub fn add_directory_artifacts(
        &mut self,
        universe: &crate::task_universe::WorkflowV2TaskUniverse,
        item: &serde_json::Value,
    ) {
        for path in super::project_artifact_contract_roots::declared_directory_paths_for_item(
            universe, item,
        ) {
            if !self.directory_artifacts.contains(&path) {
                self.directory_artifacts.push(path);
            }
        }
    }

    /// Was `path` declared as a directory by one of this run's tasks?
    ///
    /// Compared on the trailing path segments so an absolute value matches the
    /// relative declaration it came from.
    pub fn declared_as_directory(&self, path: &str) -> bool {
        let candidate = path.trim().trim_end_matches('/').replace('\\', "/");
        self.directory_artifacts.iter().any(|declared| {
            let declared = declared.trim_end_matches('/');
            !declared.is_empty()
                && (candidate == declared || candidate.ends_with(&format!("/{declared}")))
        })
    }
}
