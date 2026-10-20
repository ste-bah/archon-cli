//! Copy relative sibling inputs named by authorized checks. The repository
//! sibling aliases the site's committed checkout. Other Git siblings are
//! cloned at HEAD, so uncommitted changes are not seen; the copied HEAD is
//! verified and recorded. A non-Git sibling is copied from its current files,
//! excluding build-output directories below its root, and its evidence has no
//! commit.
use super::*;

const OMIT: &[&str] = &[".git", "target", "node_modules"];

impl super::ScratchRoots {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn project(&self) -> &Path {
        &self.project
    }

    pub fn repository(&self) -> &Path {
        &self.repository
    }

    pub fn target(&self) -> PathBuf {
        self.target.clone()
    }

    pub(crate) fn sibling_revisions(&self) -> &[crate::acceptance_sibling_paths::SiblingRevision] {
        &self.sibling_revisions
    }
}

pub(super) fn copy_external(
    project: &Path,
    repository: &Path,
    copied_project: &Path,
    copied_repository: &Path,
    checks: &[(String, crate::task_set_contract::TrustedCwd)],
    remaining: &mut u64,
) -> WorkflowResult<Vec<crate::acceptance_sibling_paths::SiblingRevision>> {
    let project = archon_shell::paths::canonicalize(project)
        .map_err(|error| WorkflowError::io(project, error))?;
    let repository = archon_shell::paths::canonicalize(repository)
        .map_err(|error| WorkflowError::io(repository, error))?;
    let mut siblings = Vec::new();
    for (check, cwd) in checks {
        let (root, copied_root) = match cwd {
            crate::task_set_contract::TrustedCwd::ProjectRoot => (&project, copied_project),
            crate::task_set_contract::TrustedCwd::RepoRoot => (&repository, copied_repository),
        };
        for sibling in crate::acceptance_sibling_paths::discover(root, std::slice::from_ref(check))
        {
            if is_project_or_ancestor(&sibling.source, &project) {
                return Err(invalid(format!(
                    "refusing to copy sibling source {} because it is the project root or an ancestor",
                    sibling.source.display()
                )));
            }
            let path = relative(root, &sibling.alias);
            siblings.push((copied_root.join(&path), path, sibling.source, *cwd));
        }
    }
    siblings.sort_by(|left, right| left.0.cmp(&right.0));
    siblings.dedup_by(|left, right| left.0 == right.0);
    let mut evidence = Vec::new();
    for (destination, relative, source, cwd) in siblings {
        let repository_relative = source.strip_prefix(&repository).ok();
        let existed = destination.exists();
        let commit = if let Some(repository_relative) = repository_relative {
            let target = copied_repository.join(repository_relative);
            if destination != target {
                let already_points_to_target = existed
                    && destination
                        .canonicalize()
                        .ok()
                        .zip(target.canonicalize().ok())
                        .is_some_and(|(destination, target)| destination == target);
                if existed && !already_points_to_target {
                    return Err(invalid(format!(
                        "sibling alias {} already exists outside the repository copy",
                        destination.display()
                    )));
                }
                if !already_points_to_target {
                    if let Some(parent) = destination.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|error| WorkflowError::io(parent, error))?;
                    }
                    link_repository_copy(&target, &destination)?;
                }
            }
            Some(git_head(copied_repository)?)
        } else if source.join(".git").exists() {
            let head = git_head(&source)?;
            if existed {
                let copied_head = git_head(&destination)?;
                let current_head = git_head(&source)?;
                if copied_head != head || current_head != head {
                    return Err(invalid(format!(
                        "sibling revision mismatch at {} (expected {head}, copied {copied_head}, current {current_head})",
                        relative.display()
                    )));
                }
                Some(head)
            } else {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| WorkflowError::io(parent, error))?;
                }
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
                        archon_shell::paths::try_plain(source.clone())
                            .map_err(|e| WorkflowError::io(&source, e))?,
                    )
                    .arg(
                        archon_shell::paths::try_plain(destination.clone())
                            .map_err(|e| WorkflowError::io(&destination, e))?,
                    )
                    .output()
                    .map_err(|error| WorkflowError::io(&source, error))?;
                if !output.status.success() {
                    return Err(invalid(format!(
                        "git clone {}: {}",
                        source.display(),
                        String::from_utf8_lossy(&output.stderr).trim()
                    )));
                }
                let copied_head = git_head(&destination)?;
                let current_head = git_head(&source)?;
                if copied_head != head || current_head != head {
                    return Err(invalid(format!(
                        "sibling revision changed while copying {} (expected {head}, copied {copied_head}, current {current_head})",
                        relative.display()
                    )));
                }
                Some(head)
            }
        } else if existed {
            None
        } else {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| WorkflowError::io(parent, error))?;
            }
            if source.is_file() {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| WorkflowError::io(parent, error))?;
                }
                super::io::copy_tree(&source, &destination, remaining, true)?;
            } else {
                copy_directory(&source, &source, &destination, remaining)?;
            }
            None
        };
        evidence.push(crate::acceptance_sibling_paths::SiblingRevision {
            path: relative,
            commit,
            cwd: Some(cwd),
        });
        if repository_relative.is_none() && !existed && source.join(".git").exists() {
            *remaining = remaining
                .checked_sub(tree_bytes(&destination)?)
                .ok_or_else(|| invalid("scratch copy exceeds configured size limit"))?;
        }
    }
    Ok(evidence)
}

fn link_repository_copy(target: &Path, destination: &Path) -> WorkflowResult<()> {
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(target, destination);
    #[cfg(windows)]
    let result = if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, destination)
    } else {
        std::os::windows::fs::symlink_file(target, destination)
    };
    result.map_err(|error| {
        WorkflowError::io(
            destination,
            std::io::Error::new(
                error.kind(),
                format!(
                    "could not link repository copy {} at sibling alias: {error}",
                    target.display()
                ),
            ),
        )
    })
}

fn is_project_or_ancestor(source: &Path, project: &Path) -> bool {
    project.starts_with(source)
}

fn git_head(path: &Path) -> WorkflowResult<String> {
    let output = archon_shell::spawn::command("git")
        .args(["-C"])
        .arg(
            archon_shell::paths::try_plain(path.to_path_buf())
                .map_err(|e| WorkflowError::io(path, e))?,
        )
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| WorkflowError::io(path, error))?;
    if !output.status.success() {
        return Err(invalid(format!(
            "git rev-parse HEAD in {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn relative(from: &Path, to: &Path) -> PathBuf {
    let left = from.components().collect::<Vec<_>>();
    let right = to.components().collect::<Vec<_>>();
    let common = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
    let mut path = PathBuf::new();
    for _ in common..left.len() {
        path.push("..");
    }
    for component in &right[common..] {
        path.push(component.as_os_str());
    }
    path
}

fn copy_directory(
    root: &Path,
    source: &Path,
    dest: &Path,
    remaining: &mut u64,
) -> WorkflowResult<()> {
    let name = source.file_name().unwrap_or_default();
    // A named sibling root is needed whole; skip build-output folders inside it.
    if source != root && OMIT.iter().any(|skip| name == *skip) {
        return Ok(());
    }
    let metadata =
        std::fs::symlink_metadata(source).map_err(|error| WorkflowError::io(source, error))?;
    let target = dest.join(source.strip_prefix(root).unwrap_or(Path::new("")));
    if metadata.is_dir() {
        std::fs::create_dir_all(&target).map_err(|error| WorkflowError::io(&target, error))?;
        for entry in std::fs::read_dir(source).map_err(|error| WorkflowError::io(source, error))? {
            let entry = entry.map_err(|error| WorkflowError::io(source, error))?;
            copy_directory(root, &entry.path(), dest, remaining)?;
        }
    } else if metadata.is_file() {
        super::io::copy_tree(source, &target, remaining, true)?;
    }
    Ok(())
}

fn tree_bytes(root: &Path) -> WorkflowResult<u64> {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).map_err(|error| WorkflowError::io(&path, error))? {
            let entry = entry.map_err(|error| WorkflowError::io(&path, error))?;
            let child = entry.path();
            let meta = entry
                .metadata()
                .map_err(|error| WorkflowError::io(&child, error))?;
            if meta.is_dir() {
                stack.push(child);
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
#[path = "acceptance_scratch_siblings_tests.rs"]
mod tests;
