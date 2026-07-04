use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    WorkflowV2Artifact, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status,
    WorkflowV2WriteSafetyError,
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

    pub fn add_artifact_requirements(&mut self, value: &serde_json::Value) {
        for path in artifact_requirement_paths(value) {
            if let Some(root) = artifact_root_from_requirement(&path) {
                push_unique_root(&mut self.artifact_roots, root);
            }
        }
    }
}

enum ProjectArtifactPath {
    Existing(String),
    Missing(String),
    NotArtifact,
}

pub fn project_artifact_context_from_v2_root(v2_root: &Path) -> WorkflowV2ProjectArtifactContext {
    let run_id = run_id_for_v2_root(v2_root);
    let artifact_roots = artifact_roots_for_run(run_id.as_deref());
    WorkflowV2ProjectArtifactContext {
        project_root: project_root_for_v2_root(v2_root).map(|path| path.display().to_string()),
        run_id,
        artifact_roots,
        policy_version: Some(PROJECT_ARTIFACT_POLICY_VERSION.to_string()),
    }
}

pub fn normalize_project_artifact_files(
    item_id: &str,
    result: &mut WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<(), WorkflowV2WriteSafetyError> {
    if context.is_empty() {
        return Ok(());
    }
    normalize_changed_project_artifacts(item_id, result, context)?;
    normalize_declared_project_artifacts(item_id, result, context)
}

fn normalize_changed_project_artifacts(
    item_id: &str,
    result: &mut WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<(), WorkflowV2WriteSafetyError> {
    if result.files_changed.is_empty() {
        return Ok(());
    }
    let mut retained = Vec::new();
    let mut artifacts = Vec::new();
    for file in std::mem::take(&mut result.files_changed) {
        match classify_project_artifact_path(item_id, &file.path, context)? {
            ProjectArtifactPath::Existing(path) => {
                artifacts.push(artifact_from_file(path, file.purpose))
            }
            ProjectArtifactPath::Missing(path) => note_missing_project_artifact(result, &path),
            ProjectArtifactPath::NotArtifact => retained.push(file),
        }
    }
    result.files_changed = retained;
    for artifact in artifacts {
        push_unique_artifact(result, artifact);
    }
    Ok(())
}

fn normalize_declared_project_artifacts(
    item_id: &str,
    result: &mut WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<(), WorkflowV2WriteSafetyError> {
    let mut retained = Vec::new();
    for mut artifact in std::mem::take(&mut result.artifacts) {
        match classify_project_artifact_path(item_id, &artifact.path, context)? {
            ProjectArtifactPath::Existing(path) => {
                artifact.path = path;
                retained.push(artifact);
            }
            ProjectArtifactPath::Missing(path) => note_missing_project_artifact(result, &path),
            ProjectArtifactPath::NotArtifact => retained.push(artifact),
        }
    }
    result.artifacts = retained;
    Ok(())
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
        Ok(ProjectArtifactPath::Existing(_) | ProjectArtifactPath::Missing(_))
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
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(ProjectArtifactPath::NotArtifact);
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
    let relative = normalize_relative_path(item_id, &relative.to_string_lossy())?;
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
    let root = context.project_root.as_deref()?.trim();
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

fn artifact_roots_for_run(run_id: Option<&str>) -> Vec<String> {
    let mut roots = vec![".archon/artifacts".to_string()];
    if let Some(run_id) = run_id.filter(|id| !id.trim().is_empty()) {
        roots.push(format!(".archon/workflows/{run_id}"));
        roots.push(format!(".archon/workflows/{run_id}/artifacts"));
    }
    roots
}

fn artifact_requirement_paths(value: &serde_json::Value) -> Vec<String> {
    let mut paths = Vec::new();
    collect_artifact_requirement_paths(value, &mut paths);
    paths
}

fn collect_artifact_requirement_paths(value: &serde_json::Value, paths: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_artifact_requirement_paths(item, paths);
            }
        }
        serde_json::Value::Object(object) => collect_artifact_object_paths(object, paths),
        serde_json::Value::String(path) => paths.push(path.clone()),
        _ => {}
    }
}

fn collect_artifact_object_paths(
    object: &serde_json::Map<String, serde_json::Value>,
    paths: &mut Vec<String>,
) {
    for key in [
        "artifact_requirements",
        "project_artifact_requirements",
        "required_artifacts",
        "artifact_paths",
        "path",
        "artifact_path",
    ] {
        if let Some(value) = object.get(key) {
            collect_artifact_requirement_paths(value, paths);
        }
    }
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
    relative.starts_with("artifacts/")
        || run_prefixed_workflow_artifact(relative, context)
        || context
            .artifact_roots
            .iter()
            .any(|root| relative_under_root(relative, root))
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

fn project_artifact_status(
    item_id: &str,
    project_root: &Path,
    relative: &str,
    output_path: String,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<ProjectArtifactPath, WorkflowV2WriteSafetyError> {
    let absolute = absolute_artifact_candidate(project_root, relative, context);
    ensure_project_path_parent_safe(item_id, project_root, &absolute, relative)?;
    if !absolute.exists() {
        return Ok(ProjectArtifactPath::Missing(output_path));
    }
    ensure_existing_project_path(item_id, project_root, &absolute, relative)?;
    Ok(ProjectArtifactPath::Existing(output_path))
}

fn ensure_existing_project_path(
    item_id: &str,
    project_root: &Path,
    absolute: &Path,
    relative: &str,
) -> Result<(), WorkflowV2WriteSafetyError> {
    let Ok(canonical_project) = std::fs::canonicalize(project_root) else {
        return Err(unsafe_target(item_id, relative));
    };
    let canonical_path =
        std::fs::canonicalize(absolute).map_err(|_| unsafe_target(item_id, relative))?;
    if canonical_path.starts_with(canonical_project) {
        Ok(())
    } else {
        Err(unsafe_target(item_id, relative))
    }
}

fn ensure_project_path_parent_safe(
    item_id: &str,
    project_root: &Path,
    absolute: &Path,
    relative: &str,
) -> Result<(), WorkflowV2WriteSafetyError> {
    let parent =
        nearest_existing_parent(absolute).ok_or_else(|| unsafe_target(item_id, relative))?;
    ensure_existing_project_path(item_id, project_root, &parent, relative)
}

fn nearest_existing_parent(path: &Path) -> Option<PathBuf> {
    let mut current = path.parent();
    while let Some(parent) = current {
        if parent.exists() {
            return Some(parent.to_path_buf());
        }
        current = parent.parent();
    }
    None
}

fn absolute_artifact_candidate(
    project_root: &Path,
    relative: &str,
    context: &WorkflowV2ProjectArtifactContext,
) -> PathBuf {
    if let Some(run_id) = context.run_id.as_deref().filter(|id| !id.is_empty()) {
        if relative.starts_with("artifacts/") {
            return project_root
                .join(".archon")
                .join("workflows")
                .join(run_id)
                .join(relative);
        }
    }
    project_root.join(relative)
}

fn clean_absolute_artifact_path(
    item_id: &str,
    raw: &str,
) -> Result<PathBuf, WorkflowV2WriteSafetyError> {
    let path = Path::new(raw);
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                clean.push(component.as_os_str())
            }
            Component::CurDir => {}
            Component::ParentDir => return Err(unsafe_target(item_id, raw)),
        }
    }
    Ok(clean)
}

fn normalize_relative_path(item_id: &str, raw: &str) -> Result<String, WorkflowV2WriteSafetyError> {
    let path = Path::new(raw.trim());
    if raw.trim().is_empty() || path.is_absolute() {
        return Err(unsafe_target(item_id, raw));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(unsafe_target(item_id, raw));
            }
        }
    }
    (!parts.is_empty())
        .then(|| parts.join("/"))
        .ok_or_else(|| unsafe_target(item_id, raw))
}

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

fn note_missing_project_artifact(result: &mut WorkflowV2Result, path: &str) {
    let id = format!("missing_project_artifact_{}", artifact_id_for_path(path));
    if !result.residual_gaps.iter().any(|gap| gap.id == id) {
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id,
            description: format!("missing project artifact evidence at {path}"),
            severity: Some("blocking".to_string()),
        });
    }
    if matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        result.status = WorkflowV2Status::NeedsReview;
    }
}

fn unsafe_target(item_id: &str, target: &str) -> WorkflowV2WriteSafetyError {
    WorkflowV2WriteSafetyError::UnsafeTarget {
        item_id: item_id.to_string(),
        target: target.to_string(),
    }
}
