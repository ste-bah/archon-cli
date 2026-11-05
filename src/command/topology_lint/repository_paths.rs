//! Containment and path faults shared by repository observation and claim checks.

use std::path::{Component, Path, PathBuf};

use archon_workflow::repository_record::{PathTruth, RepositoryTree};

#[derive(Debug)]
pub(super) struct PathFault(pub String);

impl std::fmt::Display for PathFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PathFault {}

fn fault(path: &str, reason: &str) -> anyhow::Error {
    PathFault(format!("`{path}`: {reason}")).into()
}

pub(super) fn metadata_error(path: &str, error: std::io::Error) -> anyhow::Error {
    use std::io::ErrorKind;
    // FilesystemLoop is still an unstable ErrorKind variant on the MSRV;
    // recognise its standard-library name without platform-specific errno values.
    let shape_fault = matches!(
        error.kind(),
        ErrorKind::NotFound
            | ErrorKind::NotADirectory
            | ErrorKind::IsADirectory
            | ErrorKind::InvalidInput
            | ErrorKind::InvalidFilename
    ) || format!("{:?}", error.kind()) == "FilesystemLoop";
    if shape_fault {
        fault(path, &format!("invalid repository path: {error}"))
    } else {
        anyhow::Error::new(error).context(format!("repository observation failed for {path}"))
    }
}

/// Reject lexical escapes before reading the filesystem, then resolve the
/// deepest existing ancestor. Missing descendants inherit its containment.
/// A dangling or cyclic symlink is a path fault, never an absent declaration.
pub(super) fn checked_path(tree: &RepositoryTree, named: &str) -> anyhow::Result<PathBuf> {
    let spelling = named.replace('\\', "/");
    let candidate = Path::new(&spelling);
    let relative = if candidate.is_absolute() {
        candidate
            .strip_prefix(tree.root())
            .map_err(|_| fault(named, "path leaves the repository"))?
    } else {
        candidate
    };
    if relative.components().any(|part| {
        matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(fault(named, "path leaves the repository"));
    }
    let root = tree.root().canonicalize().map_err(|error| {
        anyhow::Error::new(error).context(format!("repository root {}", tree.root().display()))
    })?;
    let joined = tree.root().join(relative);
    let mut ancestor = tree.root().to_path_buf();
    for part in relative.components() {
        ancestor.push(part.as_os_str());
        match std::fs::symlink_metadata(&ancestor) {
            Ok(_) => {
                let resolved = ancestor
                    .canonicalize()
                    .map_err(|error| metadata_error(named, error))?;
                if !resolved.starts_with(&root) {
                    return Err(fault(named, "path leaves the repository"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(joined),
            Err(error) => return Err(metadata_error(named, error)),
        }
    }
    Ok(joined)
}

pub(super) fn truth(tree: &RepositoryTree, named: &str) -> anyhow::Result<PathTruth> {
    let path = checked_path(tree, named)?;
    let in_checkout = match std::fs::metadata(path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(metadata_error(named, error)),
    };
    Ok(PathTruth {
        at_base: tree
            .relative_to_root(named)
            .is_some_and(|relative| tree.exists_at_base(&relative)),
        in_checkout,
    })
}
