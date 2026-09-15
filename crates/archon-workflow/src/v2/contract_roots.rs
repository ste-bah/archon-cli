//! The ordered directories a declared deliverable path resolves under.
//!
//! # Issue-22: the project is not the repository
//!
//! A deliverable contract's relative `artifact_path` (and `registry_path`,
//! instance globs, registry-referenced files) was joined to ONE directory, the
//! project artifact root. Live, a task declared a repository-relative source
//! file that existed in the target repository, but the run's project root is a
//! separate directory, so the host reported "declared deliverable missing or
//! empty" and demoted an accepted verification to needs_review — for every
//! task whose deliverable lives in the repository, on every remediation round,
//! with no action any agent could take.
//!
//! The fix is an ordered list, not a guess: the project artifact root first
//! (project artifacts live there and a repository copy is the anomaly — the
//! same order `artifact_presence` already stamps), then the target repository
//! root when it is known and different. A relative path is present when it
//! exists under any root, and the FIRST root it exists under is the one every
//! later predicate on that path uses. Absolute paths never consult the roots.
//! With one root the behaviour is exactly the single-root behaviour.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractRoots {
    project: String,
    repository: Option<String>,
}

impl ContractRoots {
    /// Project root first, repository root second. A repository root that is
    /// empty or names the same directory as the project root adds nothing and
    /// is dropped, so "one root" stays one root.
    pub fn new(project: impl Into<String>, repository: Option<&str>) -> Self {
        let project = project.into();
        let repository = repository
            .map(str::trim)
            .filter(|root| !root.is_empty() && !same_directory(root, &project))
            .map(str::to_string);
        Self {
            project,
            repository,
        }
    }

    pub fn project_only(project: impl Into<String>) -> Self {
        Self::new(project, None)
    }

    pub fn project(&self) -> &str {
        &self.project
    }

    pub fn repository(&self) -> Option<&str> {
        self.repository.as_deref()
    }

    /// Every root, in resolution order.
    pub fn ordered(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.project.as_str()).chain(self.repository.as_deref())
    }

    /// Where a declared path lives: absolute paths as written; a relative path
    /// under the first root it exists beneath, else under the project root so
    /// a missing file is reported against the primary location.
    pub fn resolve(&self, declared: &str) -> PathBuf {
        let path = Path::new(declared);
        if path.is_absolute() || path.has_root() {
            return path.to_path_buf();
        }
        self.ordered()
            .map(|root| Path::new(root).join(path))
            .find(|candidate| candidate.exists())
            .unwrap_or_else(|| Path::new(&self.project).join(path))
    }

    /// The roots as a diagnostic list, for "looked under a, b" failure text.
    pub fn described(&self) -> String {
        self.ordered().collect::<Vec<_>>().join(", ")
    }
}

fn same_directory(left: &str, right: &str) -> bool {
    let trim = |root: &str| root.trim_end_matches(['/', '\\']).replace('\\', "/");
    trim(left) == trim(right)
}

#[cfg(test)]
mod tests {
    use super::ContractRoots;

    #[test]
    fn a_repository_root_equal_to_the_project_root_is_dropped() {
        let roots = ContractRoots::new("/proj", Some("/proj/"));
        assert_eq!(roots.repository(), None);
        assert_eq!(roots.ordered().collect::<Vec<_>>(), vec!["/proj"]);
        assert_eq!(roots, ContractRoots::project_only("/proj"));
    }

    #[test]
    fn roots_are_ordered_project_then_repository() {
        let roots = ContractRoots::new("/proj", Some("/repo"));
        assert_eq!(roots.ordered().collect::<Vec<_>>(), vec!["/proj", "/repo"]);
        assert_eq!(roots.described(), "/proj, /repo");
    }

    #[test]
    fn a_relative_path_resolves_under_the_first_root_it_exists_beneath() {
        let project = tempfile::tempdir().expect("project");
        let repository = tempfile::tempdir().expect("repository");
        std::fs::create_dir_all(repository.path().join("src")).expect("dir");
        std::fs::write(repository.path().join("src/lib.rs"), "pub fn x() {}").expect("file");
        let roots = ContractRoots::new(
            project.path().to_str().expect("project"),
            Some(repository.path().to_str().expect("repository")),
        );
        assert_eq!(
            roots.resolve("src/lib.rs"),
            repository.path().join("src/lib.rs")
        );
        // Present under both: the project copy wins.
        std::fs::create_dir_all(project.path().join("src")).expect("dir");
        std::fs::write(project.path().join("src/lib.rs"), "shadow").expect("file");
        assert_eq!(
            roots.resolve("src/lib.rs"),
            project.path().join("src/lib.rs")
        );
        // Under neither: reported against the project root.
        assert_eq!(
            roots.resolve("src/none.rs"),
            project.path().join("src/none.rs")
        );
        // Absolute: untouched.
        assert_eq!(
            roots.resolve("/abs/x.json"),
            std::path::PathBuf::from("/abs/x.json")
        );
    }
}
