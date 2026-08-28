//! Path and identity checks the trusted command resolver depends on.

use std::path::{Component, Path};

use archon_workflow::{WorkflowError, WorkflowResult};

pub(crate) fn validate_task_id(task_id: &str) -> WorkflowResult<()> {
    let parts = task_id.split('-').collect::<Vec<_>>();
    let valid = parts.len() == 3
        && parts[0] == "TASK"
        && !parts[1].is_empty()
        && parts[1]
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        && parts[2].len() == 3
        && parts[2].chars().all(|ch| ch.is_ascii_digit());
    if valid {
        Ok(())
    } else {
        Err(WorkflowError::SpecInvalid(format!(
            "frozen task id '{task_id}' does not match TASK-<AREA>-<NNN>"
        )))
    }
}

pub(crate) fn validate_lexical_absolute(path: &Path, label: &str) -> WorkflowResult<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(WorkflowError::SpecInvalid(format!(
            "{label} {} is not a normalized absolute path",
            path.display()
        )));
    }
    Ok(())
}

/// Confine a path a command will write, whether or not it exists yet.
///
/// `validate_existing_path` canonicalises, which a destination that has never
/// been written cannot satisfy. This validates the parent the same way and then
/// refuses anything that is not a plain, non-symlink file name inside it.
pub(crate) fn validate_publication_destination(
    path: &Path,
    root: &Path,
    label: &str,
) -> WorkflowResult<()> {
    validate_lexical_absolute(path, label)?;
    validate_existing_path(root, None, "task root")?;
    let parent = path
        .parent()
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("{label} has no parent")))?;
    let canonical_parent = parent.canonicalize().map_err(|source| WorkflowError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let canonical_root = root.canonicalize().map_err(|source| WorkflowError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    if canonical_parent != canonical_root {
        return Err(WorkflowError::SpecInvalid(format!(
            "{label} {} is not a direct child of task root {}",
            path.display(),
            root.display()
        )));
    }
    if let Ok(metadata) = std::fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(WorkflowError::SpecInvalid(format!(
            "{label} {} is not a regular non-symlink file",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn validate_existing_path(
    path: &Path,
    root: Option<&Path>,
    label: &str,
) -> WorkflowResult<()> {
    validate_lexical_absolute(path, label)?;
    let canonical = path.canonicalize().map_err(|source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if let Some(root) = root {
        let canonical_root = root.canonicalize().map_err(|source| WorkflowError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        if !canonical.starts_with(&canonical_root) {
            return Err(WorkflowError::SpecInvalid(format!(
                "{label} {} escapes canonical root {} through symlink or traversal",
                path.display(),
                root.display()
            )));
        }
        if let Ok(relative) = path.strip_prefix(root) {
            let mut current = root.to_path_buf();
            for component in relative.components() {
                current.push(component.as_os_str());
                let metadata =
                    std::fs::symlink_metadata(&current).map_err(|source| WorkflowError::Io {
                        path: current.clone(),
                        source,
                    })?;
                if metadata.file_type().is_symlink() {
                    return Err(WorkflowError::SpecInvalid(format!(
                        "{label} {} descends through symlink {}",
                        path.display(),
                        current.display()
                    )));
                }
            }
        }
    }
    Ok(())
}
