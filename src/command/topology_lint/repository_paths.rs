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

/// Compare absolute spellings only against the lexical and canonical root.
/// Never probe an outside candidate (or any of its ancestors): even ENOTDIR
/// or EACCES there is unrelated to the recorded repository.
pub(super) fn relative_path(tree: &RepositoryTree, named: &str) -> anyhow::Result<Option<String>> {
    let spelling = named.replace('\\', "/");
    let candidate = Path::new(&spelling);
    if !candidate.is_absolute() || candidate.starts_with(tree.root()) {
        return Ok(tree.relative_to_root(&spelling));
    }
    let root = tree.root().canonicalize().map_err(|error| {
        anyhow::Error::new(error).context(format!("repository root {}", tree.root().display()))
    })?;
    Ok(candidate.strip_prefix(root).ok().map(|relative| {
        archon_workflow::repository_record::normalize_relative(&relative.to_string_lossy())
    }))
}

pub(super) const PATH_FORM_CODE: &str = "repository_path_not_root_relative";

/// A suffix present at the base is an author path-form fault, not an
/// existence contradiction. Keep the suggestions deterministic and bounded.
pub(super) fn path_form_finding(
    tree: &RepositoryTree,
    relative: &str,
    named: &str,
    context: &[String],
) -> Option<String> {
    let mut matches = tree.paths_ending_with(relative);
    // Parsers normalize a trailing directory slash away. A bare directory
    // suffix still names a path; bare filenames require an explicit scope.
    if matches.is_empty()
        || (!relative.contains('/') && !matches.iter().all(|path| tree.is_dir_at_base(path)))
    {
        return None;
    }
    matches.sort_by_key(|path| {
        let common = context
            .iter()
            .map(|hint| {
                path.split('/')
                    .zip(hint.split('/'))
                    .take_while(|(left, right)| left == right)
                    .count()
            })
            .max()
            .unwrap_or(0);
        (std::cmp::Reverse(common), path.split('/').count(), *path)
    });
    let remaining = matches.len().saturating_sub(3);
    let mut matches = matches
        .into_iter()
        .take(3)
        .map(|path| format!("`{path}`"))
        .collect::<Vec<_>>()
        .join(", ");
    if remaining > 0 {
        matches.push_str(&format!(", +{remaining} more"));
    }
    if Path::new(&named.replace('\\', "/")).is_absolute() {
        return Some(format!(
            "[{PATH_FORM_CODE}] `{named}`: absolute path does not exist; did you mean {matches}"
        ));
    }
    Some(format!(
        "[{PATH_FORM_CODE}] `{named}` is not repository-root relative: no such path from the repository root; write the full repository-root path (matches: {matches})"
    ))
}

/// Directory hints from the body's declared files and explicit heading scopes.
pub(super) fn suggestion_context(tree: &RepositoryTree, raw: &str) -> Vec<String> {
    let prose = super::fences::prose_lines(raw)
        .collect::<Vec<_>>()
        .join("\n");
    let items = archon_workflow::task_universe::parsing::declared_files_expected_to_change_items_preserving_lines(&prose);
    let declared = items
        .iter()
        .flat_map(|item| archon_write_plan::declared_paths::declared_file_paths(item))
        .filter_map(|path| {
            let relative = tree.relative_to_root(&path)?;
            relative.rsplit_once('/').map(|(dir, _)| dir.to_string())
        });
    let scopes = super::repository_claims::scoped_sections(raw)
        .into_iter()
        .filter_map(|(_, scope)| scope)
        .filter_map(|scope| tree.relative_to_root(&scope));
    declared.chain(scopes).collect()
}

pub(super) fn finding_code(text: &str) -> &'static str {
    if text.contains(&format!("[{PATH_FORM_CODE}]")) {
        PATH_FORM_CODE
    } else {
        "repository_claim"
    }
}

/// Reject lexical escapes before reading the filesystem, then resolve the
/// deepest existing ancestor. Missing descendants inherit its containment.
/// A dangling or cyclic symlink is a path fault, never an absent declaration.
pub(super) fn checked_path(tree: &RepositoryTree, named: &str) -> anyhow::Result<PathBuf> {
    let spelling = named.replace('\\', "/");
    let candidate = Path::new(&spelling);
    if !candidate.is_absolute()
        && candidate.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(fault(named, "path leaves the repository"));
    }
    let relative =
        relative_path(tree, named)?.ok_or_else(|| fault(named, "path leaves the repository"))?;
    let relative = Path::new(&relative);
    let root = tree.root().canonicalize().map_err(|error| {
        anyhow::Error::new(error).context(format!("repository root {}", tree.root().display()))
    })?;
    let joined = tree.root().join(relative);
    let mut ancestor = tree.root().to_path_buf();
    let mut parts = relative.components().peekable();
    while let Some(part) = parts.next() {
        ancestor.push(part.as_os_str());
        match std::fs::symlink_metadata(&ancestor) {
            Ok(_) => {
                let resolved = ancestor
                    .canonicalize()
                    .map_err(|error| metadata_error(named, error))?;
                if !resolved.starts_with(&root) {
                    return Err(fault(named, "path leaves the repository"));
                }
                // Windows may report NotFound for a child of a regular file.
                // Validate each ancestor's type before probing its descendants.
                if parts.peek().is_some()
                    && !std::fs::metadata(&resolved)
                        .map_err(|error| metadata_error(named, error))?
                        .is_dir()
                {
                    return Err(fault(
                        named,
                        "invalid repository path: ancestor is not a directory",
                    ));
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

/// Hash canonical path metadata rather than diagnostic prose or list order.
/// The opaque slot also keeps submitted paths out of the defect envelope.
pub(super) fn claim_location(paths: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut paths = paths.to_vec();
    paths.sort();
    paths.dedup();
    let mut hash = Sha256::new();
    for path in paths {
        hash.update(path.len().to_string());
        hash.update(b":");
        hash.update(path.as_bytes());
    }
    format!("claims/{}", hex::encode(hash.finalize()))
}
