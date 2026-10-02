//! Issue-124: what an isolated write branch may modify outside its worktree,
//! as the HOST names it.
//!
//! A write-branch coder rewrote a live data file under the project root from
//! its worktree, and nothing refused it. Two paths were open: the shell, which
//! `bash_write_sandbox` now bounds at the OS level from the sets computed
//! here, and the file tools, which [`WorkflowReadGuard::boundary_refusal`]
//! judges against the same sets so the two cannot disagree.
//!
//! # Where the sets come from
//!
//! Never from anything inside the area the agent may write, and never from
//! `ToolContext::write_roots`, which is empty unless the operator enabled
//! `workflow.write_confinement` and which means "writable" when it is set.
//! The write layer stamps the branch input (`_write_boundary`, see
//! `archon_workflow::agent_dispatch_port::write_boundary`) with:
//!
//! - `sealed`: the project root and the canonical checkout the worktree was
//!   branched from;
//! - `writable`: the branch's declared project artifacts, which the host
//!   judges where they are, and the canonical dependency directories whose
//!   children the worktree shares by symlink.
//!
//! Also sealed: the run store (from the run-store scope) and the shared git
//! directory of a sealed checkout that is itself a linked worktree. Also
//! writable: the worktree and the run's artifact directory. A writable entry
//! that contains a sealed root is dropped — it would re-open the root.
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::{DeclaredTargetScope, WorkflowReadGuard};

/// The host's boundary stamp, as absolute paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostWriteBoundary {
    sealed: Vec<PathBuf>,
    writable: Vec<PathBuf>,
}

impl HostWriteBoundary {
    /// Relative or empty entries are dropped: every entry is host-resolved
    /// and absolute, and a relative one would be judged against whatever
    /// directory this process happens to be in.
    pub fn new(sealed: &[String], writable: &[String]) -> Self {
        let absolute = |entries: &[String]| {
            entries
                .iter()
                .map(|entry| PathBuf::from(entry.trim()))
                .filter(|path| path.is_absolute())
                .collect::<Vec<_>>()
        };
        Self {
            sealed: absolute(sealed),
            writable: absolute(writable),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sealed.is_empty()
    }
}

/// What one isolated branch may not write (`protected`) and, inside that,
/// may (`writable`), under every spelling known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryPaths {
    pub protected: Vec<PathBuf>,
    pub writable: Vec<PathBuf>,
    /// The worktree, as the host gave it; for a read-only call, its working
    /// root, which is writable only when the host named it so.
    pub worktree: PathBuf,
    /// Drawn around a read-only call (Batch G), not a write branch.
    pub read_only: bool,
}

tokio::task_local! { static READ_ONLY_BOUNDARY: ReadOnlyBoundaryScope; }

/// Batch G: the host's write boundary for a READ-ONLY agent call (verify,
/// review, audit, adjudication, confirmation, ...).
///
/// A read-only verifier ran the product's own regenerator against the live
/// project root from its shell and rewrote a tracked project input; nothing
/// bounded it, because the Issue-124 boundary was drawn only around isolated
/// write branches. A read-only call gets the same OS boundary, stricter: the
/// host's sealed roots (the project root, the canonical checkout, the host's
/// evidence stores; the run store is added from the run-store scope) and,
/// inside them, only what the host names writable (a scratch working root
/// of its own), plus the temp, cache and target directories the host
/// selected for each command. Never the run's artifact directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOnlyBoundaryScope {
    working_root: PathBuf,
    boundary: HostWriteBoundary,
}

impl ReadOnlyBoundaryScope {
    /// `None` without an absolute working root or a sealed root: nothing to
    /// draw a boundary around.
    pub fn new(working_root: &str, sealed: &[String], writable: &[String]) -> Option<Self> {
        let working_root = PathBuf::from(working_root.trim());
        let boundary = HostWriteBoundary::new(sealed, writable);
        (working_root.is_absolute() && !boundary.is_empty()).then_some(Self {
            working_root,
            boundary,
        })
    }

    pub(super) fn into_parts(self) -> (HostWriteBoundary, PathBuf) {
        (self.boundary, self.working_root)
    }
}

/// Scope the read-only boundary for the guard built inside `work`.
pub async fn scope_read_only_boundary<T>(
    scope: Option<ReadOnlyBoundaryScope>,
    work: impl std::future::Future<Output = T>,
) -> T {
    match scope {
        Some(scope) => READ_ONLY_BOUNDARY.scope(scope, work).await,
        None => work.await,
    }
}

/// The read-only boundary in effect, if any: read once at guard construction.
pub(super) fn read_only_current() -> Option<(HostWriteBoundary, PathBuf)> {
    READ_ONLY_BOUNDARY
        .try_with(|scope| scope.clone().into_parts())
        .ok()
}

impl BoundaryPaths {
    /// Add `dir` to the writable set unless it would re-open a protected root.
    pub fn allow(&mut self, dir: &Path) {
        for spelling in spellings(dir) {
            if spelling.to_str().is_some()
                && !self
                    .protected
                    .iter()
                    .any(|root| root.starts_with(&spelling))
            {
                push_unique(&mut self.writable, spelling);
            }
        }
    }

    /// Whether a write at the absolute `path` is refused.
    pub fn refuses(&self, path: &Path) -> bool {
        spellings(path).iter().any(|candidate| {
            self.protected
                .iter()
                .any(|root| candidate.starts_with(root))
                && !self.writable.iter().any(|dir| candidate.starts_with(dir))
        })
    }
}

impl WorkflowReadGuard {
    /// The boundary for an isolated write branch, or a read-only call, whose
    /// host stamped one; `None` for every other call.
    pub fn boundary_paths(&self) -> Option<BoundaryPaths> {
        let read_only = self.read_only();
        if !self.isolated_write_branch() && !read_only {
            return None;
        }
        let stamp = self.boundary.as_ref().filter(|b| !b.is_empty())?;
        let worktree = self.worktree_root.clone()?;
        // A read-only call's working root is often the canonical checkout
        // itself: it is never exempted from the sealed roots, and is
        // writable only when the host's stamp names it.
        let tree = if read_only {
            Vec::new()
        } else {
            spellings(&worktree)
        };
        let mut protected = Vec::new();
        let store = self.run_store.iter().flat_map(|s| s.store_roots().to_vec());
        let shared_git = stamp
            .sealed
            .iter()
            .filter_map(|root| linked_common_dir(root));
        for root in stamp.sealed.iter().cloned().chain(store).chain(shared_git) {
            for spelling in spellings(&root) {
                if spelling.to_str().is_some() && !tree.iter().any(|t| spelling.starts_with(t)) {
                    push_unique(&mut protected, spelling);
                }
            }
        }
        if protected.is_empty() {
            return None;
        }
        let mut paths = BoundaryPaths {
            protected,
            writable: Vec::new(),
            worktree,
            read_only,
        };
        let artifacts = self
            .run_store
            .iter()
            .filter(|_| !read_only)
            .flat_map(|s| s.artifact_dirs().to_vec());
        for dir in tree
            .iter()
            .cloned()
            .chain(artifacts)
            .chain(stamp.writable.clone())
        {
            paths.allow(&dir);
        }
        Some(paths)
    }

    /// Whether the host's stamp names `path` writable (a declared project
    /// artifact or a shared dependency directory).
    pub(crate) fn declared_writable(&self, path: &Path) -> bool {
        let Some(stamp) = self.boundary.as_ref() else {
            return false;
        };
        let declared: Vec<PathBuf> = stamp.writable.iter().flat_map(|d| spellings(d)).collect();
        spellings(path)
            .iter()
            .any(|candidate| declared.iter().any(|dir| candidate.starts_with(dir)))
    }

    /// A file-tool write outside the branch's boundary, refused before the
    /// file changes. The same sets bound the branch's shell.
    pub(super) fn boundary_refusal(&self, name: &str, input: &Value) -> Option<String> {
        if !super::run_store::mutates_a_file(name) {
            return None;
        }
        let paths = self.boundary_paths()?;
        let named = ["file_path", "path"]
            .iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))?
            .trim();
        let absolute = if Path::new(named).is_absolute() {
            PathBuf::from(named)
        } else {
            paths.worktree.join(named)
        };
        if paths.read_only {
            return paths.refuses(&absolute).then(|| {
                format!(
                    "Error: {named} is outside what this read-only call may write. The project \
                     root, the repository checkout and the run's records are the host's; a \
                     read-only call reports what it found in its envelope and changes nothing."
                )
            });
        }
        paths.refuses(&absolute).then(|| {
            format!(
                "Error: {named} is outside this isolated write branch's worktree. The project \
                 and repository roots are the host's: it lands them from your worktree once the \
                 branch is accepted. Make the change in your worktree ({}) at the \
                 repository-relative path; a report belongs in the run's artifact directory. If \
                 the task can only be done by changing a file outside the worktree, report that \
                 as a blocker in your envelope.",
                paths.worktree.display()
            )
        })
    }
}

impl DeclaredTargetScope {
    /// The host's boundary stamp for this branch (Issue-124).
    #[must_use]
    pub fn with_write_boundary(mut self, boundary: HostWriteBoundary) -> Self {
        self.boundary = Some(boundary).filter(|b| !b.is_empty());
        self
    }

    pub(super) fn write_boundary(&self) -> Option<&HostWriteBoundary> {
        self.boundary.as_ref()
    }

    /// Whether the host's boundary names the worktree path `relative`
    /// writable: a seeded copy of the project's data, which the landing
    /// applies to the project root and git never carries (Batch E).
    pub(super) fn boundary_admits(&self, relative: &str) -> bool {
        let Some(stamp) = self.boundary.as_ref() else {
            return false;
        };
        let inside: Vec<PathBuf> = stamp
            .writable
            .iter()
            .flat_map(|dir| spellings(dir))
            .filter(|dir| self.roots.iter().any(|root| dir.starts_with(root)))
            .collect();
        self.roots.iter().any(|root| {
            let path = root.join(relative);
            inside.iter().any(|dir| path.starts_with(dir))
        })
    }

    /// The worktree root as the host gave it.
    pub(super) fn worktree_root(&self) -> Option<&PathBuf> {
        self.roots.first()
    }
}

/// The shared git directory of `checkout` when `checkout` is itself a linked
/// worktree (`.git` is a file): the branch worktrees made from it keep their
/// git state there, outside `checkout`.
fn linked_common_dir(checkout: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(checkout.join(".git")).ok()?;
    let gitdir = checkout.join(text.trim().strip_prefix("gitdir:")?.trim());
    let common = std::fs::read_to_string(gitdir.join("commondir"))
        .map(|relative| gitdir.join(relative.trim()))
        .unwrap_or(gitdir);
    Some(normalise(&common))
}

/// The git common directory of a sealed checkout: `<checkout>/.git` for an
/// ordinary clone, or the one [`linked_common_dir`] finds.
pub fn checkout_common_dir(checkout: &Path) -> Option<PathBuf> {
    let dot_git = checkout.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    linked_common_dir(checkout)
}

/// `path` lexically normalised, and with its longest existing prefix
/// canonicalised: the kernel judges the real path, and `/var` is
/// `/private/var` on macOS.
pub fn spellings(path: &Path) -> Vec<PathBuf> {
    spellings_within(path, 8)
}

fn spellings_within(path: &Path, depth: u8) -> Vec<PathBuf> {
    let given = normalise(path);
    let mut out = vec![given.clone()];
    let mut existing = given.as_path();
    let mut rest = Vec::new();
    while existing.symlink_metadata().is_err() {
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return out;
        };
        rest.push(name.to_os_string());
        existing = parent;
    }
    if let Some(mut real) = real_path(existing, depth) {
        real.extend(rest.iter().rev());
        push_unique(&mut out, real);
    }
    out
}

/// `existing` with every link resolved, including a final link that dangles:
/// a write through it lands where it points, so that is where it is judged.
fn real_path(existing: &Path, depth: u8) -> Option<PathBuf> {
    if let Ok(real) = std::fs::canonicalize(existing) {
        return Some(real);
    }
    let target = std::fs::read_link(existing).ok()?;
    let target = existing.parent()?.join(target);
    spellings_within(&target, depth.checked_sub(1)?).pop()
}

fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub(crate) fn push_unique(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.contains(&candidate) {
        paths.push(candidate);
    }
}
