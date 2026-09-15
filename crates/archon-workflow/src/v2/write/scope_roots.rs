//! The ceiling on what a write branch may be granted: the packages its plan
//! declares, and nothing beyond them.
//!
//! # Why a ceiling at all (Issue-27)
//!
//! The scope grant (`worktree_scope_grant`) widens a branch to every changed
//! path no other item in its wave claims (Issue-16). That rule has no upper
//! bound, and a single-item wave contests nothing. Live on wf-719ff3b0
//! `agents-11`: one item, TASK-DL-009, declared five targets under
//! `crates/archon-trading/`, `crates/archon-tui/` and `src/`; the coder ran
//! clippy on an unrelated crate and edited twenty files under
//! `crates/archon-workflow/` and `crates/archon-knowledge/`. Nothing claimed
//! them, so all twenty were granted, declared in the manifest, and committed
//! under a task that never mentioned either crate.
//!
//! # What the ceiling is keyed on
//!
//! Only the plan's declared targets and the repository's own package markers —
//! nothing about this repository, this workflow, or any language:
//!
//! - For each declared path, its ancestors are walked upward from the nearest
//!   one, STRICTLY below the repository root. The first ancestor holding a
//!   package manifest ([`PACKAGE_MANIFESTS`], plus `*.csproj`) is the scope
//!   root; a declared target in `crates/archon-trading/src/` makes the whole
//!   crate the branch's to change, because a change in one crate legitimately
//!   ripples through its own `lib.rs`, tests and siblings.
//! - With no manifest below the root, the top-level directory is the root:
//!   `src/command/x.rs` makes `src/` the root. That is the widest sensible
//!   reading of an unpackaged tree, and errs towards granting.
//! - A declared file directly at the repository root is an exact-file root.
//! - A declared directory scope is its own root, unless a package root
//!   contains it, in which case the package wins.
//!
//! The manifests are resolved on disk against `plan.canonical_root`, the tree
//! the worktree was created from. A root that does not exist (unit fixtures)
//! has no manifests and falls back to top-level directories.
//!
//! # What is always covered
//!
//! A candidate directly at the repository root — `Cargo.toml`, `Cargo.lock`,
//! `package-lock.json`, `README.md` — is never crate pollution: workspace
//! manifests and lockfiles are shared build files that a dependency change in
//! any crate legitimately touches. They are still contested through the wave
//! claims like any other candidate; this module only decides what reaches
//! that contest.
//!
//! A plan that declares nothing has no roots to derive, and then nothing is
//! out of scope: the ceiling is a property of the declared targets, and with
//! none declared the grant behaves exactly as it did before this module.

use std::path::Path;

use archon_write_plan::WritePlan;

/// File names that mark a directory as a package root, whatever the language.
const PACKAGE_MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "CMakeLists.txt",
    "Makefile",
    "composer.json",
    "Gemfile",
    "mix.exs",
];

/// The directories and root-level files a branch's plan reaches, resolved
/// once per branch from the declared targets and the repository's package
/// markers. Read by the grant (what may be granted) and by the preamble
/// (what the agent is told), so prompt and gate agree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ScopeRoots {
    /// Directory roots, repo-relative, no trailing slash, sorted and with
    /// nested duplicates pruned.
    dirs: Vec<String>,
    /// Declared files directly at the repository root, sorted.
    files: Vec<String>,
}

impl ScopeRoots {
    /// Whether `repo_relative` is inside the branch's scope: under a directory
    /// root (by the same boundary-aware rule `path_is_planned` uses), equal to
    /// an exact-file root, or directly at the repository root. Everything is
    /// covered when the plan declared nothing.
    pub(super) fn covers(&self, repo_relative: &str) -> bool {
        if self.is_empty() || !repo_relative.contains('/') {
            return true;
        }
        self.files.iter().any(|file| file == repo_relative)
            || self
                .dirs
                .iter()
                .any(|dir| crate::v2::write_mode::paths_overlap(dir, repo_relative))
    }

    /// No declared target produced a root.
    pub(super) fn is_empty(&self) -> bool {
        self.dirs.is_empty() && self.files.is_empty()
    }

    /// The roots as the preamble and the gap name them: directories with a
    /// trailing slash, files as they are, e.g.
    /// `crates/archon-trading/, crates/archon-tui/, src/`.
    pub(super) fn describe(&self) -> String {
        self.dirs
            .iter()
            .map(|dir| format!("{dir}/"))
            .chain(self.files.iter().cloned())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The one sentence the branch's task carries after its owned targets,
    /// so the agent is told the ceiling the gate will apply. Empty when there
    /// is no ceiling.
    pub(super) fn preamble(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        format!(
            "\nScope roots: {}. Changes outside these (except files directly at the repository \
             root) are discarded before capture and reported as a gap — do not edit other \
             crates/packages, run formatters or linters tree-wide, or fix pre-existing \
             warnings elsewhere.\n",
            self.describe()
        )
    }
}

/// The scope roots of `plan`: see the module documentation for the rules.
pub(super) fn scope_roots(plan: &WritePlan) -> ScopeRoots {
    let mut roots = ScopeRoots::default();
    for target in &plan.target_files {
        let path = target.as_str();
        match path.rsplit_once('/') {
            None => roots.files.push(path),
            Some((parent, _)) => roots.dirs.push(
                package_root(&plan.canonical_root, parent).unwrap_or_else(|| top_level(parent)),
            ),
        }
    }
    for scope in &plan.target_dir_scopes {
        let path = scope.as_str();
        roots
            .dirs
            .push(package_root(&plan.canonical_root, &path).unwrap_or(path));
    }
    roots.files.sort();
    roots.files.dedup();
    roots.dirs.sort();
    roots.dirs.dedup();
    // A root nested under another adds nothing to `covers` and would only
    // clutter what the agent is told.
    let dirs = roots.dirs.clone();
    roots.dirs.retain(|dir| {
        !dirs.iter().any(|other| {
            other != dir
                && dir
                    .strip_prefix(other.as_str())
                    .is_some_and(|s| s.starts_with('/'))
        })
    });
    roots
}

/// The nearest ancestor of `dir` (itself included) holding a package
/// manifest on disk under `canonical_root`, strictly below the root.
fn package_root(canonical_root: &Path, dir: &str) -> Option<String> {
    let mut current = dir;
    loop {
        if has_package_manifest(&canonical_root.join(current)) {
            return Some(current.to_string());
        }
        match current.rsplit_once('/') {
            Some((parent, _)) => current = parent,
            None => return None,
        }
    }
}

/// The first component of a repo-relative directory: `src` for `src/command`.
fn top_level(dir: &str) -> String {
    dir.split('/').next().unwrap_or(dir).to_string()
}

/// Whether `dir` holds one of [`PACKAGE_MANIFESTS`] or a `*.csproj`. A
/// directory that cannot be read holds none.
fn has_package_manifest(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        PACKAGE_MANIFESTS.contains(&name.as_ref()) || name.ends_with(".csproj")
    })
}

#[cfg(test)]
#[path = "scope_roots_tests.rs"]
mod tests;
