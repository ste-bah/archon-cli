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
            let Some(site_repository) = layout.site_repository.as_ref() else {
                return false;
            };
            let copied_repository = site_repository
                .canonicalize()
                .unwrap_or_else(|_| site_repository.clone());
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
    let source_root = repository.canonicalize().ok()?;
    let sibling_root = sibling.source.canonicalize().ok()?;
    let resolved = resolved.canonicalize().ok()?;
    let remainder = resolved.strip_prefix(&sibling_root).ok()?;
    if remainder.as_os_str().is_empty() {
        return None;
    }
    let source_relative = sibling_root.strip_prefix(source_root).ok()?;
    let target = layout
        .check_siblings
        .get(check_id)?
        .iter()
        .find_map(|named| {
            let target = named.target.as_ref()?;
            let canonical_target = target.canonicalize().unwrap_or_else(|_| target.clone());
            let copied_repository = layout.site_repository.as_ref()?;
            let copied_repository = copied_repository
                .canonicalize()
                .unwrap_or_else(|_| copied_repository.clone());
            (canonical_target.strip_prefix(&copied_repository).ok()? == source_relative).then(
                || {
                    (
                        lexical_absolute(&canonical_target.join(remainder)),
                        copied_repository,
                    )
                },
            )
        })?;
    let (target, copied_repository) = target;
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
    canonical_target.starts_with(&copied_repository).then(|| {
        (
            target.to_string_lossy().into_owned(),
            copied_repository.to_string_lossy().into_owned(),
        )
    })
}

pub(super) fn direct_copy_repository_input(
    path: &Path,
    base: &Path,
    cwd: &Path,
    repository: &Path,
    layout: Option<&CopyLayout>,
    check_id: &str,
) -> Option<(String, String)> {
    let layout = layout?;
    let target = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let source_root = repository.canonicalize().ok()?;
    if let Ok(target) = target.canonicalize()
        && let Ok(relative) = target.strip_prefix(&source_root)
        && !relative.as_os_str().is_empty()
        && repository.join(relative).exists()
    {
        let site_repository = layout.site_repository.as_ref()?;
        return Some((
            site_repository
                .join(relative)
                .to_string_lossy()
                .into_owned(),
            site_repository.to_string_lossy().into_owned(),
        ));
    }
    let resolved = lexical_absolute(&target);
    if layout.project.starts_with(&layout.root) && layout.repository.starts_with(&layout.root) {
        let site_cwd = if cwd == source_root {
            &layout.repository
        } else {
            &layout.project
        };
        let effective = super::paths::relative_path(cwd, base);
        let site_target = lexical_absolute(&site_cwd.join(effective).join(path));
        let copy_root = lexical_absolute(&layout.repository);
        let relative = site_target.strip_prefix(&copy_root).ok()?;
        let source = repository.join(relative).canonicalize().ok()?;
        if !relative.as_os_str().is_empty() && source.starts_with(&source_root) {
            return Some((
                site_target.to_string_lossy().into_owned(),
                copy_root.to_string_lossy().into_owned(),
            ));
        }
    }
    layout
        .check_siblings
        .get(check_id)?
        .iter()
        .find_map(|sibling| {
            let alias = lexical_absolute(&cwd.join(&sibling.path));
            let remainder = resolved.strip_prefix(&alias).ok()?;
            let target = sibling.target.as_ref()?;
            let site_target = lexical_absolute(&target.join(remainder));
            let site_root = layout.site_repository.as_ref()?;
            let site_root = site_root
                .canonicalize()
                .unwrap_or_else(|_| site_root.clone());
            let relative = site_target.strip_prefix(&site_root).ok()?;
            if relative.as_os_str().is_empty() || !repository.join(relative).exists() {
                return None;
            }
            Some((
                site_target.to_string_lossy().into_owned(),
                site_root.to_string_lossy().into_owned(),
            ))
        })
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
