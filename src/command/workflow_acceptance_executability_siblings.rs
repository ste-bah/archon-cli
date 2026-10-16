//! Materialize relative check inputs that name project siblings. A Git sibling
//! is cloned at HEAD, so uncommitted changes are not seen; the copied HEAD is
//! verified and recorded. A non-Git sibling is copied from its current files,
//! excluding build-output directories below its root, and its evidence has no
//! commit.

use std::path::{Component, Path, PathBuf};

use archon_workflow::task_set_contract::{AcceptanceCheck, AcceptanceContract};

use super::Unrun;

const SKIP: &[&str] = &[".git", "target", "node_modules"];

pub(super) fn copy_for_checks(
    copied_project: &Path,
    live_project: &Path,
    copied_repository: &Path,
    live_repository: &Path,
    contract: &AcceptanceContract,
) -> Result<Vec<super::SiblingRevision>, Unrun> {
    let project = archon_shell::paths::canonicalize(live_project)
        .map_err(|error| Unrun(format!("{}: {error}", live_project.display())))?;
    let repository = archon_shell::paths::canonicalize(live_repository)
        .map_err(|error| Unrun(format!("{}: {error}", live_repository.display())))?;
    let mut paths = Vec::new();
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
                    Unrun("could not resolve sibling path from check root".into())
                })?;
                paths.push((copied_root.join(&relative), relative, sibling.source));
            }
        }
    }
    paths.sort();
    paths.dedup_by(|a, b| a.0 == b.0);
    let mut evidence = Vec::new();
    for (destination, relative, source) in paths {
        if destination.exists() {
            continue;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| Unrun(format!("{}: {error}", parent.display())))?;
        }
        let commit = if git_repository(&source) {
            Some(clone_head(&source, &destination)?)
        } else {
            if source.is_file() {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| Unrun(format!("{}: {error}", parent.display())))?;
                }
                std::fs::copy(&source, &destination)
                    .map_err(|error| Unrun(format!("{}: {error}", destination.display())))?;
            } else {
                copy_directory(&source, &source, &destination)?;
            }
            None
        };
        evidence.push(super::SiblingRevision {
            path: relative,
            commit,
        });
    }
    Ok(evidence)
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

fn clone_head(source: &Path, destination: &Path) -> Result<String, Unrun> {
    let expected = git_head(source)?;
    let output = archon_shell::spawn::command("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "clone",
            "--quiet",
            "--shared",
            "--",
        ])
        .arg(
            archon_shell::paths::try_plain(source.to_path_buf())
                .map_err(|e| Unrun(format!("git clone {}: {e}", source.display())))?,
        )
        .arg(
            archon_shell::paths::try_plain(destination.to_path_buf())
                .map_err(|e| Unrun(format!("git clone {}: {e}", destination.display())))?,
        )
        .output()
        .map_err(|error| Unrun(format!("git clone {}: {error}", source.display())))?;
    if !output.status.success() {
        return Err(Unrun(format!(
            "git clone {}: {}",
            source.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let copied = git_head(destination)?;
    let current = git_head(source)?;
    if copied != expected || current != expected {
        return Err(Unrun(format!(
            "sibling revision changed while copying {} (expected {expected}, copied {copied}, current {current})",
            source.display()
        )));
    }
    Ok(expected)
}

fn git_head(path: &Path) -> Result<String, Unrun> {
    let output = archon_shell::spawn::command("git")
        .args(["-C"])
        .arg(
            archon_shell::paths::try_plain(path.to_path_buf())
                .map_err(|e| Unrun(format!("git rev-parse HEAD in {}: {e}", path.display())))?,
        )
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| Unrun(format!("git rev-parse HEAD in {}: {error}", path.display())))?;
    if !output.status.success() {
        return Err(Unrun(format!(
            "git rev-parse HEAD in {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn copy_directory(root: &Path, source: &Path, destination: &Path) -> Result<(), Unrun> {
    let relative = source
        .strip_prefix(root)
        .expect("copy root contains source");
    let target = destination.join(relative);
    let metadata = source
        .symlink_metadata()
        .map_err(|error| Unrun(format!("{}: {error}", source.display())))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        // A named sibling root is needed whole; skip build-output folders inside it.
        if source != root
            && SKIP
                .iter()
                .any(|name| source.file_name().is_some_and(|file| file == *name))
        {
            return Ok(());
        }
        std::fs::create_dir_all(&target)
            .map_err(|error| Unrun(format!("{}: {error}", target.display())))?;
        for item in std::fs::read_dir(source)
            .map_err(|error| Unrun(format!("{}: {error}", source.display())))?
        {
            let item = item.map_err(|error| Unrun(format!("{}: {error}", source.display())))?;
            copy_directory(root, &item.path(), destination)?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| Unrun(format!("{}: {error}", parent.display())))?;
        }
        std::fs::copy(source, &target)
            .map_err(|error| Unrun(format!("{}: {error}", target.display())))?;
    }
    Ok(())
}
