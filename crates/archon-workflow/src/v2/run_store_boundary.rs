//! Where the host keeps its own records, expressed as a project-relative path.
//!
//! A run's directory holds the engine's bookkeeping: the per-branch results,
//! the stage records, the write coordination state, the event log and the
//! checkpoint. None of it is repository source and none of it is a branch
//! deliverable, so no declared target and no discovered contract may resolve
//! into it.
//!
//! This used to be covered incidentally. The run's own directory was listed as
//! an artifact root, and "under an artifact root" was the test that kept a
//! declared path out of the repository target set. That listing was what let a
//! branch write into the run's bookkeeping in the first place
//! (`project_artifacts::artifact_roots_for_run`), and removing it would have
//! turned every path under the run directory into an admissible repository
//! target — trading one hole for a worse one. The rule is stated directly here
//! instead, so it holds whatever any run happens to advertise, and covers the
//! run-prefixed files beside the run directories that the artifact-root test
//! never reached.
//!
//! The absolute-path half of the same boundary — refusing an agent's write at
//! tool-call time — lives in `archon_tools::workflow_read_guard`, which is
//! handed the run directory by the host dispatch. This half is relative
//! because the paths it judges are declared, and a declaration is written
//! relative to the project root.

/// Project-relative directory holding every run's records.
pub(crate) const RUN_STORE_ROOT: &str = ".archon/workflows";

/// Whether `path` — project-relative, `/`-separated, already normalised —
/// names the run store or anything inside it.
///
/// The store directory itself counts: a declared target naming it is no more
/// a repository file than one naming a record inside it.
pub(crate) fn is_run_store_path(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    path == RUN_STORE_ROOT || path.starts_with(&format!("{RUN_STORE_ROOT}/"))
}

#[cfg(test)]
#[path = "run_store_boundary_tests.rs"]
mod tests;
