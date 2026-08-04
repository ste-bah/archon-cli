//! Path safety for project artifacts: confining every resolved path to the
//! project root, and rejecting the ones that escape it.
//!
//! A child module of `project_artifacts` rather than a sibling, so these stay
//! reachable only from there. Split out because each source file in this tree
//! is held under a 500-line ceiling.

use super::*;

pub(super) fn ensure_existing_project_path(
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

pub(super) fn ensure_project_path_parent_safe(
    item_id: &str,
    project_root: &Path,
    absolute: &Path,
    relative: &str,
) -> Result<(), WorkflowV2WriteSafetyError> {
    let parent =
        nearest_existing_parent(absolute).ok_or_else(|| unsafe_target(item_id, relative))?;
    ensure_existing_project_path(item_id, project_root, &parent, relative)
}

pub(super) fn nearest_existing_parent(path: &Path) -> Option<PathBuf> {
    let mut current = path.parent();
    while let Some(parent) = current {
        if parent.exists() {
            return Some(parent.to_path_buf());
        }
        current = parent.parent();
    }
    None
}

pub(super) fn absolute_artifact_candidate(
    project_root: &Path,
    relative: &str,
    context: &WorkflowV2ProjectArtifactContext,
) -> PathBuf {
    if let Some(run_id) = context.run_id.as_deref().filter(|id| !id.is_empty())
        && relative.starts_with("artifacts/")
    {
        return project_root
            .join(".archon")
            .join("workflows")
            .join(run_id)
            .join(relative);
    }
    project_root.join(relative)
}

pub(super) fn clean_absolute_artifact_path(
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

pub(super) fn normalize_relative_path(
    item_id: &str,
    raw: &str,
) -> Result<String, WorkflowV2WriteSafetyError> {
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
