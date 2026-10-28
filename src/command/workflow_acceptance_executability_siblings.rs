//! Materialize relative check inputs that name project siblings. The
//! repository sibling aliases the site's committed checkout. Other Git
//! siblings are cloned at HEAD, so uncommitted changes are not seen; the
//! copied HEAD is verified and recorded. A non-Git sibling is copied from its
//! current files, excluding build-output directories below its root, and its
//! evidence has no commit.

use std::path::{Component, Path, PathBuf};

use archon_workflow::task_set_contract::{AcceptanceCheck, AcceptanceContract};

use super::Unrun;

const SKIP: &[&str] = &[".git", "target", "node_modules"];

pub(crate) fn manifest_relative_input(token: &str, command: &str, cwd: &Path) -> Option<String> {
    let relative = Path::new(token);
    // A path that exists from the check's actual working directory belongs
    // there, even when the command also names a sibling manifest.
    if exact_path_exists(&cwd.join(relative)) {
        return None;
    }
    // A sibling manifest must not turn identifiers and prose words into
    // paths. The caller applies the ordinary candidate filters before using
    // this mapping; retain the path-shape requirement here as well.
    if !token.contains('/') && relative.extension().is_none() {
        return None;
    }
    archon_workflow::acceptance_sibling_paths::scan(command)
        .into_iter()
        .filter(|named| {
            named.required
                && named
                    .path
                    .file_name()
                    .is_some_and(|name| name == "Cargo.toml")
        })
        .find_map(|named| {
            let root = cwd.join(&named.path).parent()?.canonicalize().ok()?;
            exact_path_exists(&root.join(relative)).then(|| {
                format!(
                    "{}/{}",
                    named.path.parent().expect("manifest parent").display(),
                    token
                )
            })
        })
}

/// `Path::exists` follows the host volume's case rules. Input discovery must
/// require every path component to match the spelling in the check text.
pub(crate) fn exact_path_exists(path: &Path) -> bool {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => current.push(".."),
            Component::Normal(name) => {
                let Ok(entries) = std::fs::read_dir(if current.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    &current
                }) else {
                    return false;
                };
                if !entries.flatten().any(|entry| entry.file_name() == name) {
                    return false;
                }
                current.push(name);
            }
        }
    }
    current.exists()
}

#[cfg(test)]
pub(super) fn copy_for_checks(
    copied_project: &Path,
    live_project: &Path,
    copied_repository: &Path,
    live_repository: &Path,
    contract: &AcceptanceContract,
) -> Result<Vec<super::SiblingRevision>, Unrun> {
    copy_for_checks_with_check_map(
        copied_project,
        live_project,
        copied_repository,
        live_repository,
        contract,
    )
    .map(|(siblings, _)| siblings)
}

pub(super) fn copy_for_checks_with_check_map(
    copied_project: &Path,
    live_project: &Path,
    copied_repository: &Path,
    live_repository: &Path,
    contract: &AcceptanceContract,
) -> Result<
    (
        Vec<super::SiblingRevision>,
        std::collections::BTreeMap<String, Vec<super::SiblingRevision>>,
    ),
    Unrun,
> {
    let project = archon_shell::paths::canonicalize(live_project).map_err(|error| {
        let text = format!("{}: {error}", live_project.display());
        Unrun::error(error.into(), text)
    })?;
    let repository = archon_shell::paths::canonicalize(live_repository).map_err(|error| {
        let text = format!("{}: {error}", live_repository.display());
        Unrun::error(error.into(), text)
    })?;
    let mut paths = Vec::new();
    let mut paths_by_check = std::collections::BTreeMap::<String, Vec<PathBuf>>::new();
    for entry in contract.acceptance.iter().chain(&contract.supplementary) {
        if let AcceptanceCheck::Command { command, cwd } = &entry.check {
            let (live_root, copied_root) = match cwd {
                archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => {
                    (&project, copied_project)
                }
                archon_workflow::task_set_contract::TrustedCwd::RepoRoot => {
                    (&repository, copied_repository)
                }
            };
            for sibling in archon_workflow::acceptance_sibling_paths::discover(
                live_root,
                std::slice::from_ref(command),
            ) {
                let relative = relative_path(live_root, &sibling.alias).ok_or_else(|| {
                    Unrun::unknown("could not resolve sibling path from check root".into())
                })?;
                paths_by_check
                    .entry(entry.id.clone())
                    .or_default()
                    .push(sibling.source.clone());
                let repository_relative = sibling
                    .source
                    .strip_prefix(&repository)
                    .ok()
                    .map(Path::to_path_buf);
                paths.push((
                    copied_root.join(&relative),
                    relative,
                    sibling.source,
                    repository_relative,
                ));
            }
        }
    }
    paths.sort();
    paths.dedup_by(|a, b| a.0 == b.0);
    let mut evidence = std::collections::BTreeMap::new();
    for (destination, relative, source, repository_relative) in paths {
        let commit = if let Some(repository_relative) = repository_relative {
            let target = copied_repository.join(repository_relative);
            if destination != target {
                let already_points_to_target = destination.exists()
                    && destination
                        .canonicalize()
                        .ok()
                        .zip(target.canonicalize().ok())
                        .is_some_and(|(destination, target)| destination == target);
                if destination.exists() && !already_points_to_target {
                    return Err(Unrun::unknown(format!(
                        "sibling alias {} conflicts with the repository copy",
                        destination.display()
                    )));
                }
                if !already_points_to_target {
                    if let Some(parent) = destination.parent() {
                        std::fs::create_dir_all(parent).map_err(|error| {
                            let text = format!("{}: {error}", parent.display());
                            Unrun::error(error.into(), text)
                        })?;
                    }
                    link_repository_copy(&target, &destination)?;
                }
            }
            Some(git_head(copied_repository)?)
        } else if destination.exists() {
            None
        } else {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    let text = format!("{}: {error}", parent.display());
                    Unrun::error(error.into(), text)
                })?;
            }
            if git_repository(&source) {
                Some(clone_head(&source, &destination)?)
            } else {
                if source.is_file() {
                    if let Some(parent) = destination.parent() {
                        std::fs::create_dir_all(parent).map_err(|error| {
                            let text = format!("{}: {error}", parent.display());
                            Unrun::error(error.into(), text)
                        })?;
                    }
                    std::fs::copy(&source, &destination).map_err(|error| {
                        let text = format!("{}: {error}", destination.display());
                        Unrun::error(error.into(), text)
                    })?;
                } else {
                    copy_directory(&source, &source, &destination)?;
                }
                None
            }
        };
        evidence
            .entry(source.clone())
            .or_insert_with(|| super::SiblingRevision {
                path: relative,
                commit,
                target: destination.canonicalize().ok(),
                site_repository: Some(copied_repository.to_path_buf()),
            });
    }
    let revisions = evidence;
    let per_check = paths_by_check
        .into_iter()
        .map(|(id, paths)| {
            let siblings = paths
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .filter_map(|source| revisions.get(&source).cloned())
                .collect();
            (id, siblings)
        })
        .collect();
    Ok((revisions.into_values().collect(), per_check))
}

pub(super) fn install_copy_layout(
    layout: &mut super::CopyLayout,
    copied_project: &Path,
    live_project: &Path,
    copied_repository: &Path,
    live_repository: &Path,
    contract: &AcceptanceContract,
) -> Result<(), Unrun> {
    let (siblings, check_siblings) = copy_for_checks_with_check_map(
        copied_project,
        live_project,
        copied_repository,
        live_repository,
        contract,
    )?;
    layout.siblings = siblings;
    layout.check_siblings = check_siblings;
    Ok(())
}

fn relative_path(from: &Path, to: &Path) -> Option<PathBuf> {
    let left: Vec<_> = from.components().collect();
    let right: Vec<_> = to.components().collect();
    let shared = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
    let mut relative = PathBuf::new();
    for _ in shared..left.len() {
        relative.push("..");
    }
    for component in &right[shared..] {
        relative.push(component.as_os_str());
    }
    Some(relative)
}

fn lexical(path: PathBuf) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    Some(result)
}

pub(crate) fn missing_for_checks(
    project: &Path,
    repository: &Path,
    contract: &AcceptanceContract,
    ids: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeMap<String, String> {
    let mut missing = std::collections::BTreeMap::new();
    let Ok(project) = archon_shell::paths::canonicalize(project) else {
        return missing;
    };
    let Ok(repository) = archon_shell::paths::canonicalize(repository) else {
        return missing;
    };
    for entry in contract.acceptance.iter().chain(&contract.supplementary) {
        if !ids.contains(&entry.id) {
            continue;
        }
        if let AcceptanceCheck::Command { command, cwd } = &entry.check {
            let root = match cwd {
                archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => &project,
                archon_workflow::task_set_contract::TrustedCwd::RepoRoot => &repository,
            };
            for named in archon_workflow::acceptance_sibling_paths::scan(command)
                .into_iter()
                .filter(|path| path.required)
            {
                let Some(path) = lexical(root.join(&named.path)) else {
                    continue;
                };
                if !path.starts_with(root) && !path.exists() {
                    missing.insert(
                        entry.id.clone(),
                        format!(
                            "check {} names missing sibling path {}",
                            entry.id,
                            named.path.display()
                        ),
                    );
                    break;
                }
            }
        }
    }
    missing
}

pub(crate) fn copied_root_names(
    project: &Path,
    repository: &Path,
    contract: &AcceptanceContract,
) -> Vec<String> {
    let Ok(project) = archon_shell::paths::canonicalize(project) else {
        return Vec::new();
    };
    let Ok(repository) = archon_shell::paths::canonicalize(repository) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in contract.acceptance.iter().chain(&contract.supplementary) {
        if let AcceptanceCheck::Command { command, cwd } = &entry.check {
            let root = match cwd {
                archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => &project,
                archon_workflow::task_set_contract::TrustedCwd::RepoRoot => &repository,
            };
            for sibling in archon_workflow::acceptance_sibling_paths::discover(
                root,
                std::slice::from_ref(command),
            ) {
                if let Some(relative) = relative_path(root, &sibling.alias)
                    && let Some(name) = sibling_root_name(&relative)
                {
                    names.push(name);
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn sibling_root_name(relative: &Path) -> Option<String> {
    relative.components().find_map(|component| match component {
        Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
        Component::Prefix(_) | Component::RootDir | Component::CurDir | Component::ParentDir => {
            None
        }
    })
}

fn git_repository(path: &Path) -> bool {
    path.join(".git").exists()
}

#[path = "workflow_acceptance_executability_sibling_io.rs"]
mod io;
use io::{clone_head, copy_directory, git_head, link_repository_copy};
