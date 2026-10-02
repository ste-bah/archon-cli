//! Issue-223: the roots stored data lives under, read from the run's own
//! records and never from a path convention.
//!
//! A run declares where its data lives twice over: the acceptance policy's
//! project inputs, and every artifact its task set declares (artifact
//! requirements, deliverable contracts and their registries and instance
//! sources), whose directory is a data root. A declaration is resolved the
//! way the run resolves the artifact itself -- relative to the project root,
//! or absolute -- and becomes a root only when:
//!
//! - it has no `..` component, so no declaration climbs out of where it
//!   names;
//! - it exists, and resolves (every link followed) inside the project root
//!   or the repository, the only two trees a landing can write;
//! - it is neither of those roots itself, which would declare everything.
//!
//! A file is stored data when, every link on its path resolved, it lies
//! under one of these roots: a link that points out of a root, or a `..`
//! that climbs out of one, resolves outside it and is never covered. The
//! `.archon/<namespace>/` project-data rule stays the default beside them
//! (`residual_paths::project_data`); with no declaration, it is the only
//! rule.
//!
//! Issue-226: an absolute declaration outside both trees is a root too, but
//! only inside a directory the run's policy allowlists
//! (`project_inputs::EXTERNAL_ROOTS_KEY`); its files are `External` stored
//! data, named by their canonical absolute path. With no allowlist, none is.

use std::path::{Component, Path, PathBuf};

use super::ScopeGrantRoot;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::write_coordinator::project_inputs::ProjectInputPolicy;
use crate::write_coordinator::project_inputs::external::resolved;

/// Most files [`DeclaredDataRoots::project_files`] lists.
pub const MAX_LISTED_FILES: usize = 50_000;

/// The data roots one run's records declare, canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredDataRoots {
    project: PathBuf,
    repository: Option<PathBuf>,
    roots: Vec<PathBuf>,
    /// Issue-226: declared roots outside both trees, each inside an
    /// allowlisted external data directory.
    external: Vec<PathBuf>,
}

/// Every path the task set declares an artifact at, as declared.
fn declared_artifacts(universe: &WorkflowV2TaskUniverse) -> impl Iterator<Item = String> + '_ {
    universe.tasks.iter().flat_map(|task| {
        (task.artifact_requirements.iter().cloned()).chain(
            task.deliverable_contracts.iter().flat_map(|contract| {
                std::iter::once(contract.artifact_path.clone())
                    .chain(contract.registry_path.clone())
                    .chain(contract.instance_source_path.clone())
            }),
        )
    })
}

impl DeclaredDataRoots {
    /// The roots `policy` and `universe` declare, `repository` beside the
    /// project as the only other tree a landing writes.
    pub fn read(
        policy: &ProjectInputPolicy,
        universe: &WorkflowV2TaskUniverse,
        repository: &Path,
    ) -> Self {
        let project = policy.project.clone();
        let repository = repository
            .canonicalize()
            .map(archon_shell::paths::plain)
            .ok();
        let inputs = policy.inputs.iter().cloned();
        let directories = declared_artifacts(universe)
            .filter_map(|raw| Path::new(raw.trim()).parent().map(Path::to_path_buf));
        let mut roots: Vec<PathBuf> = inputs
            .chain(directories)
            .filter(|declared| {
                !declared.as_os_str().is_empty()
                    && !declared.components().any(|c| c == Component::ParentDir)
            })
            .filter_map(|declared| {
                project
                    .join(declared)
                    .canonicalize()
                    .map(archon_shell::paths::plain)
                    .ok()
            })
            .filter(|root| {
                let inside = |tree: &Path| root.starts_with(tree) && root.as_path() != tree;
                let at_a_tree = Some(root) == repository.as_ref() || root == &project;
                !at_a_tree && (inside(&project) || repository.as_deref().is_some_and(inside))
            })
            .collect();
        roots.sort();
        roots.dedup();
        let mut external: Vec<PathBuf> = declared_artifacts(universe)
            .filter_map(|raw| Path::new(raw.trim()).parent().map(Path::to_path_buf))
            .filter(|declared| {
                declared.is_absolute() && !declared.components().any(|c| c == Component::ParentDir)
            })
            .filter_map(|declared| declared.canonicalize().map(archon_shell::paths::plain).ok())
            .filter(|root| {
                let allowed = policy.external.allowed();
                allowed.iter().any(|tree| root.starts_with(tree))
                    && !root.starts_with(&project)
                    && !repository
                        .as_ref()
                        .is_some_and(|repo| root.starts_with(repo))
            })
            .collect();
        external.sort();
        external.dedup();
        Self {
            project,
            repository,
            roots,
            external,
        }
    }

    /// Whether `path` (absolute) is its own resolved path and lies under a
    /// declared external root (Issue-226).
    pub fn covers_external(&self, path: &Path) -> bool {
        resolved(path).as_deref() == Some(path)
            && self.external.iter().any(|root| path.starts_with(root))
    }

    /// Where the existing file `candidate` (absolute, or relative to the
    /// project root) is stored data, every link resolved: its path relative
    /// to the tree it lands in, and that tree -- whichever of the project
    /// root and the repository holds it more closely. `None` when it is no
    /// file under a declared root.
    pub fn locate(&self, candidate: &Path) -> Option<(String, ScopeGrantRoot)> {
        let path = self
            .project
            .join(candidate)
            .canonicalize()
            .map(archon_shell::paths::plain)
            .ok()?;
        if path.is_file() && self.external.iter().any(|root| path.starts_with(root)) {
            return Some((path.to_str()?.to_string(), ScopeGrantRoot::External));
        }
        if !path.is_file() || !self.roots.iter().any(|root| path.starts_with(root)) {
            return None;
        }
        let in_project = path.starts_with(&self.project);
        let repository = (self.repository.as_ref())
            .filter(|repo| path.starts_with(repo))
            .filter(|repo| {
                !in_project || (repo.starts_with(&self.project) && *repo != &self.project)
            });
        let (tree, root) = match repository {
            Some(repo) => (repo.as_path(), ScopeGrantRoot::Repository),
            None if in_project => (self.project.as_path(), ScopeGrantRoot::Project),
            None => return None,
        };
        let relative = path.strip_prefix(tree).ok()?.to_str()?.replace('\\', "/");
        Some((relative, root))
    }

    /// Every regular file under the declared roots that lands in the
    /// project ([`Self::locate`]), project-relative, sorted: links are never
    /// followed, and the walk stops at [`MAX_LISTED_FILES`].
    pub fn project_files(&self) -> Vec<String> {
        let mut files = std::collections::BTreeSet::new();
        let mut stack: Vec<PathBuf> = self.roots.clone();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                if files.len() >= MAX_LISTED_FILES {
                    return files.into_iter().collect();
                }
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() => stack.push(entry.path()),
                    Ok(kind) if kind.is_file() => {
                        if let Some((rel, ScopeGrantRoot::Project)) = self.locate(&entry.path()) {
                            files.insert(rel);
                        }
                    }
                    _ => {}
                }
            }
        }
        files.into_iter().collect()
    }

    /// Whether project-relative `rel` lies under a declared root inside the
    /// project and is itself the path it resolves to: no link anywhere on
    /// it, so a write there stays where it was declared.
    pub fn covers_project(&self, rel: &str) -> bool {
        let lexical = self.project.join(rel);
        resolved(&lexical).as_ref() == Some(&lexical)
            && (self.roots.iter())
                .any(|root| root.starts_with(&self.project) && lexical.starts_with(root))
    }
}
