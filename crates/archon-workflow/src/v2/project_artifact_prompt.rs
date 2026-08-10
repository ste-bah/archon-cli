use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::artifact_path_guard::{
    ArtifactPathRejection, expand_project_root_template, validate_declared_artifact_path,
};
use super::host_api::WorkflowV2ArtifactRequirement;
use super::project_artifact_contract::artifact_requirement_paths;
use super::project_artifacts::WorkflowV2ProjectArtifactContext;

pub(crate) fn project_artifact_prompt_section(
    input: &Value,
    required_artifacts: &[WorkflowV2ArtifactRequirement],
    write_capable: bool,
    context: &WorkflowV2ProjectArtifactContext,
) -> String {
    let declared = declared_project_artifacts(input, required_artifacts, context);
    if declared.is_empty() {
        return String::new();
    }
    let mut section = String::new();
    if !declared.entries.is_empty() {
        section.push_str(
            "\n## Resolved Project Artifact Paths\n\
             Use these absolute paths for project artifacts. Do not resolve relative `.archon/...` \
             artifact paths against `repository_root`.\n",
        );
        for (raw, absolute) in &declared.entries {
            section.push_str(&format!("- {raw} => {absolute}\n"));
        }
        if write_capable {
            section.push_str(
                "These paths are this call's declared artifact contract: write every file listed \
                 above and include each path in your structured result's `artifacts` array. A \
                 declared artifact that does not exist as a non-empty regular file on return \
                 fails this call — a directory of that name does not satisfy it.\n",
            );
        }
    }
    if !declared.refused.is_empty() {
        section.push_str(REFUSED_HEADING);
        for (raw, reason) in &declared.refused {
            section.push_str(&format!("- {raw} => REFUSED: {reason}\n"));
        }
    }
    section
}

const REFUSED_HEADING: &str = "\n## Refused Declared Artifact Paths\n\
     The following declared values are NOT paths and were refused. Do not create a file or \
     directory named after any of them, and do not pass one to `mkdir`, a redirect, or any other \
     path position. They are prose or unexpanded templates; treat them as description only.\n";

/// The declared artifact contract for a call, split into what survived
/// validation and what was refused.
pub(crate) struct DeclaredProjectArtifacts {
    /// `(as declared, resolved absolute)` for every value that is a path.
    pub entries: Vec<(String, String)>,
    /// `(as declared, why refused)` for every value that is not.
    pub refused: Vec<(String, String)>,
}

impl DeclaredProjectArtifacts {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.refused.is_empty()
    }
}

/// The declared artifact contract for a call: the explicit
/// `required_artifacts` declaration plus explicit artifact-requirement fields
/// in the call input, resolved against the project artifact root. This same
/// set drives both the agent prompt and the on-return validation, so a path
/// is only ever validated if the agent was instructed to produce it.
///
/// # Issue #168: the description never reaches the path position
///
/// This is the single choke point where a declared value becomes a path handed
/// to an agent. `required_artifacts` arrives from the workflow script's
/// `requiredArtifacts` option, which accepted any non-empty string — including
/// an acceptance criterion. Joined to the project root and printed under
/// "Resolved Project Artifact Paths ... write every file listed above", a
/// sentence containing `/` is an instruction to `mkdir -p` a nested tree, which
/// is exactly the litter run `wf-67dd2599` left behind.
///
/// So every value is expanded and validated here, before it is written into a
/// prompt and before it is checked on return. A refusal is carried out of this
/// function rather than dropped: the prompt tells the agent the value is not a
/// path, and the completion check fails the call.
pub(crate) fn declared_project_artifacts(
    input: &Value,
    required_artifacts: &[WorkflowV2ArtifactRequirement],
    context: &WorkflowV2ProjectArtifactContext,
) -> DeclaredProjectArtifacts {
    let mut declared = DeclaredProjectArtifacts {
        entries: Vec::new(),
        refused: Vec::new(),
    };
    let Some(root) = context
        .project_root
        .as_deref()
        .filter(|root| !root.is_empty())
    else {
        return declared;
    };
    let mut raw_paths: Vec<String> = required_artifacts
        .iter()
        .map(|requirement| requirement.path.clone())
        .collect();
    raw_paths.extend(artifact_requirement_paths(input));
    let mut seen_entry = BTreeSet::new();
    let mut seen_refusal = BTreeSet::new();
    for raw in raw_paths {
        match checked_project_artifact_path(root, &raw) {
            Ok(Some(entry)) => {
                if seen_entry.insert(entry.clone()) {
                    declared.entries.push(entry);
                }
            }
            Ok(None) => {}
            Err(rejection) => {
                let refusal = (raw.trim().to_string(), rejection.to_string());
                if seen_refusal.insert(refusal.clone()) {
                    declared.refused.push(refusal);
                }
            }
        }
    }
    declared
}

/// Expand, validate, then resolve. `Ok(None)` means "not a project artifact"
/// (an absolute path outside the project root); `Err` means "not a path".
fn checked_project_artifact_path(
    root: &str,
    raw: &str,
) -> Result<Option<(String, String)>, ArtifactPathRejection> {
    let expanded = expand_project_root_template(raw, Some(root))?;
    let validated = validate_declared_artifact_path(&expanded)?;
    if path_has_parent_component(&validated) {
        return Ok(None);
    }
    let path = Path::new(&validated);
    if path.is_absolute() {
        return Ok(path
            .starts_with(root)
            .then(|| (validated.clone(), validated.clone())));
    }
    Ok(Some((
        validated.clone(),
        PathBuf::from(root).join(path).display().to_string(),
    )))
}

fn path_has_parent_component(raw: &str) -> bool {
    Path::new(raw)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
}
