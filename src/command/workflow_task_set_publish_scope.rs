//! Destination authority supplied by the caller, never by journal contents.
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};

pub(crate) type Scopes = [(PathBuf, Option<String>)];

/// A directory without a name authorizes its own subtree; a named scope
/// authorizes exactly that file. Resolve the trusted root and refuse traversal
/// and symlinks below it, including a symlink at the destination itself.
pub(crate) fn validate_destination(path: &Path, scopes: &Scopes) -> Result<PathBuf> {
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(anyhow!(
            "publication path {} contains traversal",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("publication path has no parent"))?
        .canonicalize()
        .with_context(|| format!("resolving parent of publication target {}", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| anyhow!("publication path has no file name"))?;
    for (root, only) in scopes {
        let canonical = match root.canonicalize() {
            Ok(root) => root,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("resolving publication root {}", root.display()));
            }
        };
        if !parent.starts_with(&canonical)
            || only
                .as_ref()
                .is_some_and(|only| parent != canonical || name != std::ffi::OsStr::new(only))
        {
            continue;
        }
        // A caller may spell the trusted root via an OS alias (/var on macOS).
        // Locate that same canonical root in this path, then check every child
        // component and the root itself before trusting the destination.
        let anchor = path
            .ancestors()
            .skip(1)
            .find(|ancestor| ancestor.canonicalize().ok().as_ref() == Some(&canonical))
            .ok_or_else(|| anyhow!("publication path {} has no authorized root", path.display()))?;
        validate_existing_parents(path, anchor)?;
        return Ok(parent.join(name));
    }
    Err(anyhow!(
        "{} is not an authorized task-set publication destination",
        path.display()
    ))
}

pub(crate) fn validate_existing_parents(target: &Path, root: &Path) -> Result<()> {
    if std::fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err(anyhow!("publication root {} is a symlink", root.display()));
    }
    let relative = target.strip_prefix(root)?;
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(anyhow!("publication path contains traversal"));
        }
        cursor.push(component);
        match std::fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(anyhow!("publication path has symlink {}", cursor.display()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
