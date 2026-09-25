use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    WorkflowV2Artifact, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2ResidualGap,
    WorkflowV2Result, WorkflowV2Status, WorkflowV2WriteSafetyError,
    artifact_path_guard::declared_artifact_defect,
    project_artifact_contract::{artifact_path_is_templated, artifact_requirement_paths},
};

pub const PROJECT_ARTIFACT_POLICY_VERSION: &str = "workflow-v2-project-artifacts-v2";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2ProjectArtifactContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_roots: Vec<String>,
    /// Exact deliverable paths an artifact-only item may write, taken from the
    /// host-parsed task universe. Matched exactly, never as a prefix.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_paths: Vec<String>,
    /// Declared deliverables the TASK wrote as directories, normalised without
    /// the trailing separator.
    ///
    /// Carried as its own list because the separator does not survive the
    /// journey: `admissible_path` rebuilds a path with `segments.join("/")`,
    /// `Path::join(..).display()` drops it again, and by the time a value
    /// reaches the completion check it is an absolute path with no way to tell
    /// a declared directory from a declared file. One live task declares
    /// `<project-data>/coverage/history/` and was failed three times
    /// for "is a directory, not the declared file" — including once after a fix
    /// that read the separator off the string, which by then was gone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directory_artifacts: Vec<String>,
    /// Where repository SOURCE lives, when that is a different tree from the
    /// project artifact root. Existence checks only — write confinement still
    /// answers to the project root alone. See `project_artifact_completion`
    /// for why the second candidate exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_evidence_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,
}

impl WorkflowV2ProjectArtifactContext {
    pub fn is_empty(&self) -> bool {
        self.project_root
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
    }

    /// Admit what the call's artifact requirements name: a requirement's
    /// directory, or — inside the run store — only its exact path. See
    /// [`store_admission`] for why the store is different.
    pub fn add_artifact_requirements(&mut self, value: &serde_json::Value) {
        for path in artifact_requirement_paths(value) {
            match store_admission::requirement_admission(&path, self.run_id.as_deref()) {
                RequirementAdmission::Root(root) => {
                    push_unique_root(&mut self.artifact_roots, root)
                }
                RequirementAdmission::Exact(path) => {
                    push_unique_root(&mut self.artifact_paths, path)
                }
                RequirementAdmission::Nothing => {}
            }
        }
    }
}

enum ProjectArtifactPath {
    Existing(String),
    /// The declared path and why it is not evidence — absent, an empty file,
    /// or a directory holding nothing. See [`project_artifact_status`].
    Missing(String, &'static str),
    Templated(String),
    NotArtifact,
}

pub fn project_artifact_context_from_v2_root(v2_root: &Path) -> WorkflowV2ProjectArtifactContext {
    let run_id = run_id_for_v2_root(v2_root);
    let artifact_roots = artifact_roots_for_run(run_id.as_deref());
    WorkflowV2ProjectArtifactContext {
        project_root: project_root_for_v2_root(v2_root).map(|path| path.display().to_string()),
        run_id,
        artifact_roots,
        artifact_paths: Vec::new(),
        directory_artifacts: Vec::new(),
        repository_root: None,
        branch_evidence_root: Some(v2_root.join("branches").display().to_string()),
        policy_version: Some(PROJECT_ARTIFACT_POLICY_VERSION.to_string()),
    }
}

/// Returns every declared path the host could not find, as the caller wrote
/// it plus the defect, so a write-capable caller can hand the exact claims back
/// to their author. The result itself still carries the matching blocking gaps.
pub fn normalize_project_artifact_files(
    item_id: &str,
    result: &mut WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<Vec<String>, WorkflowV2WriteSafetyError> {
    if context.is_empty() {
        return Ok(Vec::new());
    }
    let mut absent = normalize_changed_project_artifacts(item_id, result, context)?;
    absent.extend(normalize_declared_project_artifacts(
        item_id, result, context,
    )?);
    // One absent path is one claim. These are two passes over two lists —
    // `files_changed` and `artifacts` — that describe the same paths, and an
    // envelope naming a deliverable in both (the normal way to report a file
    // you changed AND delivered) concatenated the identical sentence twice
    // into one error, so the repair prompt asked for the same single file two
    // times. Folded on the whole rendered claim, which carries the path and
    // the defect together, so two genuinely different defects on one path
    // both survive; first-seen order is kept.
    let mut seen = std::collections::HashSet::new();
    absent.retain(|claim| seen.insert(claim.clone()));
    Ok(absent)
}

fn normalize_changed_project_artifacts(
    item_id: &str,
    result: &mut WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<Vec<String>, WorkflowV2WriteSafetyError> {
    let mut absent = Vec::new();
    if result.files_changed.is_empty() {
        return Ok(absent);
    }
    let mut retained = Vec::new();
    let mut artifacts = Vec::new();
    for file in std::mem::take(&mut result.files_changed) {
        match classify_project_artifact_path(item_id, &file.path, context)? {
            ProjectArtifactPath::Existing(path) => {
                artifacts.push(artifact_from_file(path, file.purpose))
            }
            ProjectArtifactPath::Missing(path, defect) => {
                absent.push(format!("{}: it {defect}", file.path));
                note_missing_project_artifact(result, &path, defect)
            }
            ProjectArtifactPath::Templated(path) => note_templated_project_artifact(result, &path),
            ProjectArtifactPath::NotArtifact => retained.push(file),
        }
    }
    result.files_changed = retained;
    for artifact in artifacts {
        push_unique_artifact(result, artifact);
    }
    Ok(absent)
}

fn normalize_declared_project_artifacts(
    item_id: &str,
    result: &mut WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<Vec<String>, WorkflowV2WriteSafetyError> {
    let mut absent = Vec::new();
    let mut retained = Vec::new();
    for mut artifact in std::mem::take(&mut result.artifacts) {
        match classify_project_artifact_path(item_id, &artifact.path, context)? {
            ProjectArtifactPath::Existing(path) => {
                artifact.path = path;
                retained.push(artifact);
            }
            ProjectArtifactPath::Missing(path, defect) => {
                absent.push(format!("{}: it {defect}", artifact.path));
                note_missing_project_artifact(result, &path, defect)
            }
            ProjectArtifactPath::Templated(path) => note_templated_project_artifact(result, &path),
            ProjectArtifactPath::NotArtifact => retained.push(artifact),
        }
    }
    result.artifacts = retained;
    Ok(absent)
}

pub fn has_project_artifact_evidence(
    result: &WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
) -> bool {
    result.artifacts.iter().any(|artifact| {
        matches!(
            classify_project_artifact_path("artifact", &artifact.path, context),
            Ok(ProjectArtifactPath::Existing(_))
        )
    })
}

pub fn has_project_artifact_requirement(
    value: &serde_json::Value,
    context: &WorkflowV2ProjectArtifactContext,
) -> bool {
    !context.is_empty()
        && artifact_requirement_paths(value)
            .iter()
            .any(|path| allowed_project_artifact_requirement(path, context))
}

fn allowed_project_artifact_requirement(
    path: &str,
    context: &WorkflowV2ProjectArtifactContext,
) -> bool {
    matches!(
        classify_project_artifact_path("artifact-requirement", path, context),
        Ok(ProjectArtifactPath::Existing(_) | ProjectArtifactPath::Missing(..))
    )
}

fn classify_project_artifact_path(
    item_id: &str,
    raw: &str,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<ProjectArtifactPath, WorkflowV2WriteSafetyError> {
    let Some(project_root) = project_root_path(context) else {
        return Ok(ProjectArtifactPath::NotArtifact);
    };
    let trimmed = strip_verbatim_prefix(raw.trim());
    if trimmed.is_empty() {
        return Ok(ProjectArtifactPath::NotArtifact);
    }
    if artifact_path_is_templated(trimmed) {
        return templated_artifact_path(item_id, trimmed, &project_root, context);
    }
    if Path::new(trimmed).is_absolute() {
        return absolute_artifact_path(item_id, trimmed, &project_root, context);
    }
    relative_artifact_path(item_id, trimmed, &project_root, context)
}

fn absolute_artifact_path(
    item_id: &str,
    raw: &str,
    project_root: &Path,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<ProjectArtifactPath, WorkflowV2WriteSafetyError> {
    let clean = clean_absolute_artifact_path(item_id, raw)?;
    let Ok(relative) = clean.strip_prefix(project_root) else {
        return Ok(ProjectArtifactPath::NotArtifact);
    };
    // `/` separators whatever the host uses: a Windows backslash path never
    // matched the `/`-separated declared paths, so a written deliverable was
    // classified NotArtifact and write-ownership rejected the branch.
    let relative =
        normalize_relative_path(item_id, &relative.to_string_lossy().replace('\\', "/"))?;
    if !allowed_relative_artifact(&relative, context) {
        return Ok(ProjectArtifactPath::NotArtifact);
    }
    project_artifact_status(
        item_id,
        project_root,
        &relative,
        clean.display().to_string(),
        context,
    )
}

fn relative_artifact_path(
    item_id: &str,
    raw: &str,
    project_root: &Path,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<ProjectArtifactPath, WorkflowV2WriteSafetyError> {
    let relative = normalize_relative_path(item_id, raw)?;
    if !allowed_relative_artifact(&relative, context) {
        return Ok(ProjectArtifactPath::NotArtifact);
    }
    project_artifact_status(item_id, project_root, &relative, relative.clone(), context)
}

fn project_root_path(context: &WorkflowV2ProjectArtifactContext) -> Option<PathBuf> {
    let root = strip_verbatim_prefix(context.project_root.as_deref()?.trim());
    (!root.is_empty()).then(|| PathBuf::from(root))
}

fn run_id_for_v2_root(v2_root: &Path) -> Option<String> {
    if v2_root.file_name().and_then(|name| name.to_str()) == Some("v2") {
        return v2_root
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(str::to_string);
    }
    v2_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

fn project_root_for_v2_root(v2_root: &Path) -> Option<PathBuf> {
    let mut current = Some(v2_root);
    while let Some(path) = current {
        if path.file_name().and_then(|name| name.to_str()) == Some(".archon") {
            return path.parent().map(Path::to_path_buf);
        }
        current = path.parent();
    }
    None
}

/// The directories an agent is told it may write artifacts into, relative to
/// the project root.
///
/// The run's own directory is NOT one of them, and its absence is the point.
/// It used to be listed beside the artifact subdirectory inside it, so the
/// context handed to every call advertised the whole run directory — the
/// per-branch results, the stage records, the coordination state, the event
/// log — as a legitimate place to put a deliverable. An agent that wrote
/// there was not escaping anything; it was doing what the advertisement said.
/// Live, a branch hand-authored a record of its own into the per-branch
/// results directory, and the host read it as one of its own on the input
/// path of every stage that followed.
///
/// `<run>/artifacts` was, and remains, the run-scoped place a deliverable
/// belongs, so nothing legitimate is lost by naming only it. Two other rules
/// admit the paths this list no longer covers: a deliverable the TASK
/// declares is matched exactly through `artifact_paths`, and a run-prefixed
/// file beside the run directory through `run_prefixed_workflow_artifact`.
/// [`crate::v2::run_store_boundary`] keeps the run directory out of the
/// repository target set now that no artifact root covers it.
fn artifact_roots_for_run(run_id: Option<&str>) -> Vec<String> {
    let mut roots = vec![".archon/artifacts".to_string()];
    if let Some(run_id) = run_id.filter(|id| !id.trim().is_empty()) {
        roots.push(format!(".archon/workflows/{run_id}/artifacts"));
    }
    roots
}

fn artifact_root_from_requirement(path: &str) -> Option<String> {
    let normalized = normalize_relative_path("artifact-requirement", path).ok()?;
    if !normalized.starts_with(".archon/") && !normalized.starts_with("artifacts/") {
        return None;
    }
    let parts = normalized
        .split('/')
        .take_while(|part| !part.contains('*') && !part.contains('<'))
        .collect::<Vec<_>>();
    if parts.len() <= 1 {
        return None;
    }
    Some(parts[..parts.len() - 1].join("/"))
}

fn push_unique_root(roots: &mut Vec<String>, root: String) {
    if !roots.iter().any(|existing| existing == &root) {
        roots.push(root);
    }
}

fn allowed_relative_artifact(relative: &str, context: &WorkflowV2ProjectArtifactContext) -> bool {
    context
        .artifact_paths
        .iter()
        .any(|declared| declared == relative)
        || relative.starts_with("artifacts/")
        || namespaced_project_data_artifact(relative)
        || run_prefixed_workflow_artifact(relative, context)
        || context
            .artifact_roots
            .iter()
            .any(|root| relative_under_root(relative, root))
}

fn namespaced_project_data_artifact(relative: &str) -> bool {
    let parts = relative.split('/').collect::<Vec<_>>();
    if parts.len() < 4 || parts[0] != ".archon" || parts[2] != "data" {
        return false;
    }
    !matches!(parts[1], "" | "artifacts" | "workflows")
}

fn run_prefixed_workflow_artifact(
    relative: &str,
    context: &WorkflowV2ProjectArtifactContext,
) -> bool {
    let Some(run_id) = context.run_id.as_deref() else {
        return false;
    };
    let Some(name) = relative.strip_prefix(".archon/workflows/") else {
        return false;
    };
    name.starts_with(&format!("{run_id}-")) && !name.contains('/')
}

fn relative_under_root(relative: &str, root: &str) -> bool {
    let Ok(root) = normalize_relative_path("artifact-root", root) else {
        return false;
    };
    relative != root && relative.starts_with(&format!("{root}/"))
}

/// Is this declared path satisfied on disk?
///
/// # Issue #168: `exists()` was the hole
///
/// This asked `absolute.exists()`. `Path::exists` answers yes for a directory,
/// and yes for a zero-byte file. Run `wf-67dd2599` created directories in the
/// project root named after acceptance criteria; a directory named after the
/// criterion that demands an artifact would have answered the check for that
/// artifact, and the run would have recorded `Existing` — artifact evidence
/// present — against something containing nothing. That is issue #153's
/// fabricated-success shape arriving through the filesystem instead of through
/// a subsystem.
///
/// Evidence is now a regular, non-empty file or a directory holding one
/// (issue-68: a contract may name a directory of run records, and a coder's
/// `runs/<id>/` full of output is not the litter above). A candidate that
/// exists but evidences nothing is reported as `Missing` naming what it
/// actually is — "is an empty file", "is an empty directory", "is a directory
/// holding no non-empty file" — rather than the misleading "missing".
fn project_artifact_status(
    item_id: &str,
    project_root: &Path,
    relative: &str,
    output_path: String,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<ProjectArtifactPath, WorkflowV2WriteSafetyError> {
    let absolute = absolute_artifact_candidate(project_root, relative, context);
    ensure_project_path_parent_safe(item_id, project_root, &absolute, relative)?;
    if let Some(defect) =
        declared_artifact_defect(relative, &absolute, context.declared_as_directory(relative))
    {
        if branch_produced_artifact(item_id, project_root, relative, context) {
            return Ok(ProjectArtifactPath::Existing(output_path));
        }
        return Ok(ProjectArtifactPath::Missing(output_path, defect));
    }
    ensure_existing_project_path(item_id, project_root, &absolute, relative)?;
    Ok(ProjectArtifactPath::Existing(output_path))
}

/// Path confinement helpers. See [`paths`] for why they are a separate file.
#[path = "project_artifacts_paths.rs"]
mod paths;
#[path = "project_artifact_store_admission.rs"]
mod store_admission;
use paths::{
    absolute_artifact_candidate, branch_produced_artifact, clean_absolute_artifact_path,
    ensure_existing_project_path, ensure_project_path_parent_safe, normalize_relative_path,
    strip_verbatim_prefix,
};
use store_admission::RequirementAdmission;
pub use store_admission::project_artifact_write_admitted;

fn artifact_from_file(path: String, purpose: Option<String>) -> WorkflowV2Artifact {
    WorkflowV2Artifact {
        id: artifact_id_for_path(&path),
        path,
        description: purpose,
    }
}

fn artifact_id_for_path(path: &str) -> String {
    let mut id = path
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    id = id.trim_matches('-').to_string();
    if id.is_empty() {
        "project-artifact".to_string()
    } else {
        id
    }
}

fn push_unique_artifact(result: &mut WorkflowV2Result, artifact: WorkflowV2Artifact) {
    if result
        .artifacts
        .iter()
        .any(|existing| existing.path == artifact.path)
    {
        return;
    }
    result.artifacts.push(artifact);
}

include!("project_artifacts_templated.rs");

#[cfg(test)]
#[path = "project_artifacts_branch_root_tests.rs"]
mod branch_root_tests;
