use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use super::write_mode::WorkflowV2WriteSafetyError;

pub fn normalize_targets_for_repository(
    item_id: &str,
    targets: &[String],
    repository_root: Option<&str>,
) -> Result<Vec<String>, WorkflowV2WriteSafetyError> {
    if targets.is_empty() {
        return Err(WorkflowV2WriteSafetyError::MissingOwnership(
            item_id.to_string(),
        ));
    }
    let mut normalized = BTreeSet::new();
    for target in targets {
        normalized.insert(normalize_target_for_repository(
            item_id,
            target,
            repository_root,
        )?);
    }
    Ok(normalized.into_iter().collect())
}

pub fn normalize_target_for_repository(
    item_id: &str,
    target: &str,
    repository_root: Option<&str>,
) -> Result<String, WorkflowV2WriteSafetyError> {
    let trimmed = target.trim();
    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return normalize_target(item_id, trimmed);
    }
    let Some(root) = repository_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
    else {
        return Err(unsafe_target(item_id, target));
    };
    normalize_absolute_target(item_id, target, path, root)
}

fn normalize_absolute_target(
    item_id: &str,
    target: &str,
    path: &Path,
    root: &str,
) -> Result<String, WorkflowV2WriteSafetyError> {
    let root_path = Path::new(root);
    if !root_path.is_absolute() {
        return Err(unsafe_target(item_id, target));
    }
    let clean_root = clean_absolute_path(item_id, root, root_path)?;
    let clean_target = clean_absolute_path(item_id, target, path)?;
    let relative = clean_target
        .strip_prefix(&clean_root)
        .map_err(|_| unsafe_target(item_id, target))?;
    if relative.as_os_str().is_empty() {
        return Err(unsafe_target(item_id, target));
    }
    normalize_target(item_id, &relative.to_string_lossy().replace('\\', "/"))
}

fn normalize_target(item_id: &str, target: &str) -> Result<String, WorkflowV2WriteSafetyError> {
    let trimmed = target.trim();
    let path = Path::new(trimmed);
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) || path.is_absolute() {
        return Err(unsafe_target(item_id, target));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(unsafe_target(item_id, target));
            }
        }
    }
    if parts.is_empty() {
        return Err(unsafe_target(item_id, target));
    }
    Ok(parts.join("/"))
}

fn clean_absolute_path(
    item_id: &str,
    original: &str,
    path: &Path,
) -> Result<PathBuf, WorkflowV2WriteSafetyError> {
    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => cleaned.push(prefix.as_os_str()),
            Component::RootDir => cleaned.push(Path::new("/")),
            Component::CurDir => {}
            Component::Normal(part) => cleaned.push(part),
            Component::ParentDir => return Err(unsafe_target(item_id, original)),
        }
    }
    Ok(cleaned)
}

fn unsafe_target(item_id: &str, target: &str) -> WorkflowV2WriteSafetyError {
    WorkflowV2WriteSafetyError::UnsafeTarget {
        item_id: item_id.to_string(),
        target: target.to_string(),
    }
}
