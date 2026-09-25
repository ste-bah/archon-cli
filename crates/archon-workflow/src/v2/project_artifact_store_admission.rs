//! What the project-artifact rules admit inside the run store, stated once.
//!
//! Two consumers ask the same question. The completion check asks whether a
//! path an agent reports is a deliverable; the host's write guard asks whether
//! a write into the run store may proceed. They used to answer separately, and
//! disagreed: the run-prefixed report beside the run directory
//! (`.archon/workflows/<run>-*.json`) was admitted as a deliverable and refused
//! as a write. [`project_artifact_write_admitted`] is the one answer both use.
//!
//! Sharing it made one breadth visible. A requirement path used to contribute
//! its parent directory as an artifact ROOT, so a requirement naming
//! `.archon/workflows/<run>/x.md` advertised the whole run directory — the
//! per-branch results and stage records included — and one naming
//! `.archon/x.md` advertised `.archon`, the whole store beneath it. That is
//! the advertisement `artifact_roots_for_run` withdrew, arriving by another
//! door. Inside the store a requirement now admits exactly the path it names,
//! and only within the current run; everywhere else it still admits its
//! parent directory, as before.

use super::WorkflowV2ProjectArtifactContext;
use crate::v2::run_store_boundary::is_run_store_path;

/// Whether the project-artifact rules admit `path` — absolute, or relative to
/// the project root — as a deliverable of the call `context` describes.
///
/// Exactly the rule the completion check applies to a reported path. A write
/// guard that admits this and nothing more cannot refuse a deliverable the
/// completion check would then demand, and cannot admit anything it would
/// not.
pub fn project_artifact_write_admitted(
    context: &WorkflowV2ProjectArtifactContext,
    path: &str,
) -> bool {
    !context.is_empty() && super::allowed_project_artifact_requirement(path, context)
}

/// How one requirement path widens the context.
pub(super) enum RequirementAdmission {
    /// Everything under this directory.
    Root(String),
    /// This one path.
    Exact(String),
    Nothing,
}

pub(super) fn requirement_admission(path: &str, run_id: Option<&str>) -> RequirementAdmission {
    let Some(root) = super::artifact_root_from_requirement(path) else {
        return RequirementAdmission::Nothing;
    };
    if root != ".archon" && !is_run_store_path(&root) {
        return RequirementAdmission::Root(root);
    }
    let Ok(exact) = super::normalize_relative_path("artifact-requirement", path) else {
        return RequirementAdmission::Nothing;
    };
    if !is_run_store_path(&exact) {
        return RequirementAdmission::Exact(exact);
    }
    let in_current_run = run_id
        .filter(|id| !id.trim().is_empty())
        .is_some_and(|id| exact.starts_with(&format!(".archon/workflows/{id}/")));
    if in_current_run {
        RequirementAdmission::Exact(exact)
    } else {
        RequirementAdmission::Nothing
    }
}

#[cfg(test)]
#[path = "project_artifact_store_admission_tests.rs"]
mod tests;
