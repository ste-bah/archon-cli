use std::path::{Path, PathBuf};

use super::super::hermetic::{CopyLayout, SiblingRevision};

pub(super) fn is_repository_source(
    sibling: &archon_workflow::acceptance_sibling_paths::SiblingSource,
    repository: &Path,
) -> bool {
    let repository = repository
        .canonicalize()
        .unwrap_or_else(|_| repository.to_path_buf());
    let source = sibling
        .source
        .canonicalize()
        .unwrap_or_else(|_| sibling.source.clone());
    source.starts_with(repository)
}

/// The copy layout records the alias the check site actually installed.
/// Resolve that alias and require its target to sit inside the copied repo.
pub(super) fn is_copy_alias(
    sibling: &archon_workflow::acceptance_sibling_paths::SiblingSource,
    repository: &Path,
    layout: Option<&CopyLayout>,
    check_id: &str,
) -> bool {
    let Some(layout) = layout else {
        return false;
    };
    let repository = repository
        .canonicalize()
        .unwrap_or_else(|_| repository.to_path_buf());
    let copied_repository = layout
        .repository
        .canonicalize()
        .unwrap_or_else(|_| layout.repository.clone());
    let source = sibling
        .source
        .canonicalize()
        .unwrap_or_else(|_| sibling.source.clone());
    let Ok(source_relative) = source.strip_prefix(&repository) else {
        return false;
    };
    layout
        .check_siblings
        .get(check_id)
        .into_iter()
        .flatten()
        .any(|named: &SiblingRevision| {
            let Some(target) = &named.target else {
                return false;
            };
            let target = target.canonicalize().unwrap_or_else(|_| target.clone());
            let Ok(target_relative) = target.strip_prefix(&copied_repository) else {
                return false;
            };
            target_relative == source_relative
        })
}

pub(super) fn canonical_move_path(
    sibling: &archon_workflow::acceptance_sibling_paths::SiblingSource,
    resolved: &Path,
    repository: &Path,
    layout: Option<&CopyLayout>,
    check_id: &str,
) -> Option<(String, String)> {
    let layout = layout?;
    let copy_root = layout
        .repository
        .canonicalize()
        .unwrap_or_else(|_| layout.repository.clone());
    let source_root = repository.canonicalize().ok()?;
    let sibling_root = sibling.source.canonicalize().ok()?;
    let resolved = resolved.canonicalize().ok()?;
    let remainder = resolved.strip_prefix(&sibling_root).ok()?;
    let source_relative = sibling_root.strip_prefix(source_root).ok()?;
    let target = layout
        .check_siblings
        .get(check_id)?
        .iter()
        .find_map(|named| {
            let target = named.target.as_ref()?;
            let canonical_target = target.canonicalize().unwrap_or_else(|_| target.clone());
            (canonical_target.strip_prefix(&copy_root).ok()? == source_relative)
                .then(|| lexical_absolute(&target.join(remainder)))
        })?;
    let canonical_target = target
        .canonicalize()
        .or_else(|_| {
            let parent = target
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .ok_or_else(|| std::io::Error::other("target parent unavailable"))?;
            let leaf = target
                .file_name()
                .ok_or_else(|| std::io::Error::other("target leaf unavailable"))?;
            Ok::<PathBuf, std::io::Error>(parent.join(leaf))
        })
        .unwrap_or_else(|_| lexical_absolute(&target));
    canonical_target.starts_with(&copy_root).then(|| {
        (
            target.to_string_lossy().into_owned(),
            layout.repository.to_string_lossy().into_owned(),
        )
    })
}

pub(super) fn direct_copy_repository_input(
    path: &Path,
    cwd: &Path,
    repository: &Path,
    layout: Option<&CopyLayout>,
) -> Option<(String, String)> {
    let layout = layout?;
    let copy_root = layout
        .repository
        .canonicalize()
        .unwrap_or_else(|_| layout.repository.clone());
    let copy_cwd = if cwd == repository.canonicalize().ok()? {
        &layout.repository
    } else {
        &layout.project
    };
    let target = if path.is_absolute() {
        path.to_path_buf()
    } else {
        copy_cwd.join(path)
    };
    let target = lexical_absolute(&target);
    let relative = match target.strip_prefix(&copy_root) {
        Ok(relative) => relative,
        Err(_) => return None,
    };
    let source = repository.canonicalize().ok()?.join(relative);
    source.canonicalize().ok()?;
    Some((
        target.to_string_lossy().into_owned(),
        copy_root.to_string_lossy().into_owned(),
    ))
}

fn lexical_absolute(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}
