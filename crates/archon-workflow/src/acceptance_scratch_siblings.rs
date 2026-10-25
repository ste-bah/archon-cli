//! Copy relative sibling inputs named by authorized checks. The repository
//! sibling aliases the site's committed checkout. Other Git siblings are
//! cloned at HEAD, so uncommitted changes are not seen; the copied HEAD is
//! verified and recorded. A non-Git sibling is copied from its current files,
//! excluding build-output directories below its root, and its evidence has no
//! commit.
use super::*;

const OMIT: &[&str] = &[".git", "target", "node_modules"];

impl super::ScratchRoots {
    pub(super) fn command_environment(
        &self,
        policy: &super::ScratchPolicy,
    ) -> BTreeMap<String, String> {
        let mut env = self.environment(policy);
        env.extend(self.host_environment.clone());
        if let Ok(root) = std::fs::canonicalize(&self.root) {
            env.insert(
                "ARCHON_SCRATCH_SITE_ROOT".into(),
                root.to_string_lossy().into_owned(),
            );
        }
        env
    }

    pub(super) fn mutation_root(&self) -> &super::mutation_fd::SiteRoot {
        &self.mutation_root
    }

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
        &self.siblings.revisions
    }

    /// Why the check at `index` of this site's checks has no copy of a
    /// sibling it names (`copy_external`), if it has none.
    pub(super) fn sibling_failure(
        &self,
        index: usize,
    ) -> Option<&(String, Option<super::faults::FaultCause>)> {
        self.siblings.failures.get(&index)
    }

    /// Before this site is restored for another check (`isolate`): see
    /// `cache::Lease::forget_unless_idle`. Without a build cache every site
    /// has its own root and build output, so nothing is shared.
    pub(super) fn forget_unless_idle(&mut self) -> WorkflowResult<()> {
        let slot = self.root.clone();
        match &mut self.lease {
            Some(lease) => lease.forget_unless_idle(&slot),
            None => Ok(()),
        }
    }
}

/// What one site copied of the siblings its checks name: the copied
/// siblings' revisions, and the result of each check (by its index in the
/// site's checks) whose sibling could not be copied.
#[derive(Default)]
pub(super) struct Copied {
    pub(super) revisions: Vec<crate::acceptance_sibling_paths::SiblingRevision>,
    pub(super) failures: BTreeMap<usize, (String, Option<super::faults::FaultCause>)>,
}

/// Copy each sibling the checks name. Siblings are copied one by one: a
/// sibling that cannot be copied is removed from the site and is the result
/// of the checks that name it, never of the batch (a keep-going build fails
/// only the targets that depend on what failed). A sibling only one check
/// names, whose copy failed on its own path (too large, not a directory, a
/// collision with the site), is that check's fault; any other failure, or
/// a sibling several checks share, leaves those checks host-unproven. Only
/// a failure to undo a partial copy, a stall or a cancellation is the
/// batch's.
pub(super) fn copy_external(
    project: &Path,
    repository: &Path,
    copied_project: &Path,
    copied_repository: &Path,
    checks: &[(String, crate::task_set_contract::TrustedCwd)],
    remaining: &mut u64,
) -> WorkflowResult<Copied> {
    let project = archon_shell::paths::canonicalize(project)
        .map_err(|error| WorkflowError::io(project, error))?;
    let repository = archon_shell::paths::canonicalize(repository)
        .map_err(|error| WorkflowError::io(repository, error))?;
    let mut siblings = Vec::new();
    for (index, (check, cwd)) in checks.iter().enumerate() {
        let (root, copied_root) = match cwd {
            crate::task_set_contract::TrustedCwd::ProjectRoot => (&project, copied_project),
            crate::task_set_contract::TrustedCwd::RepoRoot => (&repository, copied_repository),
        };
        for sibling in crate::acceptance_sibling_paths::discover(root, std::slice::from_ref(check))
        {
            // Callers resolve each check first (`refused_sibling`); this
            // guards the copy itself.
            if is_project_or_ancestor(&sibling.source, &project) {
                return Err(invalid(format!(
                    "refusing to copy sibling source {} because it is the project root or an ancestor",
                    sibling.source.display()
                )));
            }
            let path = relative(root, &sibling.alias);
            siblings.push((copied_root.join(&path), path, sibling.source, *cwd, index));
        }
    }
    siblings.sort_by(|left, right| left.0.cmp(&right.0).then(left.4.cmp(&right.4)));
    let mut copied = Copied::default();
    let mut siblings = siblings.into_iter().peekable();
    while let Some((destination, relative, source, cwd, index)) = siblings.next() {
        let mut named_by = vec![index];
        while let Some(next) = siblings.next_if(|next| next.0 == destination) {
            if !named_by.contains(&next.4) {
                named_by.push(next.4);
            }
        }
        let existed = destination.exists();
        let budget = *remaining;
        let sibling = Sibling {
            destination: &destination,
            relative: &relative,
            source: &source,
            existed,
        };
        let copy = copy_one(&sibling, &repository, copied_repository, remaining);
        match copy {
            Ok(commit) => copied
                .revisions
                .push(crate::acceptance_sibling_paths::SiblingRevision {
                    path: relative,
                    commit,
                    cwd: Some(cwd),
                }),
            Err(error) => {
                if matches!(
                    error,
                    WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)
                ) {
                    return Err(error);
                }
                super::control::check()?;
                *remaining = budget;
                if !existed && destination.symlink_metadata().is_ok() {
                    super::io::remove_owned_tree(&destination)?;
                }
                let result = sibling_failure(&error, &relative, named_by.len() == 1);
                for index in named_by {
                    copied.failures.insert(index, result.clone());
                }
            }
        }
    }
    Ok(copied)
}

struct Sibling<'a> {
    destination: &'a Path,
    relative: &'a Path,
    source: &'a Path,
    existed: bool,
}

/// The result of the checks naming a sibling at `relative` whose copy failed
/// with `error`: the check's own fault when it alone names it and the cause
/// is in its path (with its typed cause), else host-unproven.
fn sibling_failure(
    error: &WorkflowError,
    relative: &Path,
    alone: bool,
) -> (String, Option<super::faults::FaultCause>) {
    use super::faults::{FaultCause, SIBLING_SHAPE_REFUSED, SIBLING_TOO_LARGE};
    let text = error.to_string();
    let own = if matches!(error, WorkflowError::ScratchCopyTooLarge) {
        Some((SIBLING_TOO_LARGE, FaultCause::SiblingTooLarge))
    } else if matches!(error, WorkflowError::ScratchCopyShape(_))
        || matches!(error, WorkflowError::Io { source, .. }
            if matches!(source.kind(), std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotADirectory | std::io::ErrorKind::IsADirectory))
    {
        Some((SIBLING_SHAPE_REFUSED, FaultCause::SiblingShapeRefused))
    } else {
        None
    };
    match own {
        Some((fault, cause)) if alone => (
            format!("{fault}: {} ({text})", relative.display()),
            Some(cause),
        ),
        _ => (
            format!(
                "sibling {} could not be copied; check is host-unproven: {text}",
                relative.display()
            ),
            None,
        ),
    }
}

/// Copy one sibling to its destination, returning its recorded revision.
fn copy_one(
    sibling: &Sibling<'_>,
    repository: &Path,
    copied_repository: &Path,
    remaining: &mut u64,
) -> WorkflowResult<Option<String>> {
    let Sibling {
        destination,
        relative,
        source,
        existed,
    } = *sibling;
    let repository_relative = source.strip_prefix(repository).ok();
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
                return Err(WorkflowError::ScratchCopyShape(format!(
                    "sibling alias {} conflicts with the repository copy",
                    destination.display()
                )));
            }
            // A previously materialized sibling alias can make this
            // nested destination resolve to the repository copy itself.
            // In that case the alias already works; creating its parent
            // would manufacture repository/target before the build link
            // guard gets to validate it.
            let destination_is_target = resolves_to_same_path(destination, &target);
            if !already_points_to_target && !destination_is_target {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| WorkflowError::io(parent, error))?;
                }
                link_repository_copy(&target, destination)?;
            }
        }
        Some(git_head(copied_repository)?)
    } else if source.join(".git").exists() {
        let head = git_head(source)?;
        if existed {
            let copied_head = git_head(destination)?;
            let current_head = git_head(source)?;
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
                    archon_shell::paths::try_plain(source.to_path_buf())
                        .map_err(|e| WorkflowError::io(source, e))?,
                )
                .arg(
                    archon_shell::paths::try_plain(destination.to_path_buf())
                        .map_err(|e| WorkflowError::io(destination, e))?,
                )
                .output()
                .map_err(|error| WorkflowError::io(source, error))?;
            if !output.status.success() {
                return Err(invalid(format!(
                    "git clone {}: {}",
                    source.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            let copied_head = git_head(destination)?;
            let current_head = git_head(source)?;
            if copied_head != head || current_head != head {
                return Err(invalid(format!(
                    "sibling revision changed while copying {} (expected {head}, copied {copied_head}, current {current_head})",
                    relative.display()
                )));
            }
            *remaining = remaining
                .checked_sub(tree_bytes(destination)?)
                .ok_or(WorkflowError::ScratchCopyTooLarge)?;
            Some(head)
        }
    } else if existed {
        None
    } else {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| WorkflowError::io(parent, error))?;
        }
        if source.is_file() {
            super::io::copy_sibling_tree(source, destination, remaining, true)?;
        } else {
            copy_directory(source, source, destination, remaining)?;
        }
        None
    };
    Ok(commit)
}

fn resolves_to_same_path(left: &Path, right: &Path) -> bool {
    fn resolve_missing_tail(path: &Path) -> Option<PathBuf> {
        let mut ancestor = path;
        let mut tail = Vec::new();
        while std::fs::symlink_metadata(ancestor).is_err() {
            tail.push(ancestor.file_name()?.to_os_string());
            ancestor = ancestor.parent()?;
        }
        let mut resolved = std::fs::canonicalize(ancestor).ok()?;
        for component in tail.iter().rev() {
            resolved.push(component);
        }
        Some(resolved)
    }

    resolve_missing_tail(left)
        .zip(resolve_missing_tail(right))
        .is_some_and(|(left, right)| left == right)
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

/// Why `check`, run at `cwd`, names a sibling no site copies -- the project
/// root or an ancestor of it -- resolved for that one check before any site
/// is prepared, so the path is that check's own finding and never voids the
/// sites of the checks beside it (each check analysed as its own target).
/// `None` when it names none, or the roots cannot be read
/// (preparing the site then reports that).
pub(crate) fn refused_sibling(
    project: &Path,
    repository: &Path,
    check: &str,
    cwd: crate::task_set_contract::TrustedCwd,
) -> Option<String> {
    let project = archon_shell::paths::canonicalize(project).ok()?;
    let repository = archon_shell::paths::canonicalize(repository).ok()?;
    let root = match cwd {
        crate::task_set_contract::TrustedCwd::ProjectRoot => &project,
        crate::task_set_contract::TrustedCwd::RepoRoot => &repository,
    };
    crate::acceptance_sibling_paths::discover(root, &[check.to_string()])
        .into_iter()
        .find(|sibling| is_project_or_ancestor(&sibling.source, &project))
        .map(|sibling| {
            format!(
                "{}: {} resolves to {}",
                super::faults::SIBLING_ROOT_REFUSED,
                relative(root, &sibling.alias).display(),
                sibling.source.display()
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

pub(crate) fn relative(from: &Path, to: &Path) -> PathBuf {
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
        super::io::copy_sibling_tree(source, &target, remaining, true)?;
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
