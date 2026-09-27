//! Obs-123: refusing a recursive shell search that would walk the run
//! store or leave the call's workspace (the roots come from
//! `shell::recursive_searches`).
use std::path::{Component, Path, PathBuf};

use super::RunStoreScope;

impl RunStoreScope {
    /// Obs-123: the refusal for a recursive shell search whose root would
    /// walk the run store (the store itself, a directory holding it such as
    /// the project root, or host records inside it) or leaves the call's
    /// workspace. Inside the workspace, the run's own agent-owned trees
    /// (artifacts, planted worktrees) and scratch directories are searchable.
    /// A root that cannot be placed (after `cd` to an unknown directory) is
    /// not judged: like the rest of the guard this is an efficiency rule, not
    /// a sandbox.
    pub(super) fn search_refusal(&self, command: &str) -> Option<String> {
        for search in super::super::shell::recursive_searches(command) {
            for root in search.roots.iter().flatten() {
                let Some(resolved) = self.resolve(root) else {
                    continue;
                };
                let within = |dir: &PathBuf| resolved == *dir || resolved.starts_with(dir);
                // A search that skips hidden directories never enters a
                // store below one.
                let walks = |store: &PathBuf| {
                    store.starts_with(&resolved)
                        && !(search.skips_hidden && hidden_between(&resolved, store))
                };
                let reason = if self.roots.iter().any(walks) {
                    "contains the run store, which holds every run ever kept on this machine \
                     (their worktrees and records)"
                } else if self.holds_host_records(&resolved) {
                    "is the host's own record directory"
                } else if self.working_root.as_ref().is_some_and(|work| !within(work))
                    && !self.exempt.iter().any(within)
                    && !scratch(&resolved)
                {
                    "is outside your workspace"
                } else {
                    continue;
                };
                let workspace = self
                    .working_root
                    .as_ref()
                    .map(|root| {
                        let holds_store = self.roots.iter().any(|store| store.starts_with(root));
                        format!(
                            " Search {} {} instead, naming the narrowest path you need, or use \
                             the Grep or Glob tool, which never walk the host's records.",
                            if holds_store {
                                "a subdirectory of your workspace that does not hold the run \
                                 store, under"
                            } else {
                                "your workspace"
                            },
                            root.display()
                        )
                    })
                    .unwrap_or_default();
                return Some(format!(
                    "`{}` searches {root} recursively, which {reason}: such a walk can run for \
                     tens of minutes and finds nothing of yours.{workspace} The host hands you \
                     what it recorded about your task in your prompt; do not search its store \
                     for it.",
                    search.program
                ));
            }
        }
        None
    }
}

/// Whether some directory strictly below `root` on the way down to `store`
/// (the store itself included) is hidden: a search that skips hidden
/// directories then never reaches `store`.
fn hidden_between(root: &Path, store: &Path) -> bool {
    store.strip_prefix(root).is_ok_and(|rest| {
        rest.components().any(|part| {
            matches!(part, Component::Normal(name) if name.to_string_lossy().starts_with('.'))
        })
    })
}

/// A scratch directory: searching one is never a walk of the store.
fn scratch(path: &Path) -> bool {
    #[cfg(windows)]
    {
        // Windows has no POSIX /tmp. Compare the host's real scratch root
        // under both ordinary and verbatim spellings, after the store checks.
        let temp = std::env::temp_dir();
        let path = archon_write_plan::lexical_path::portable(&path.to_string_lossy());
        return [Some(temp.clone()), temp.canonicalize().ok()]
            .into_iter()
            .flatten()
            .any(|root| {
                let root = archon_write_plan::lexical_path::portable(&root.to_string_lossy());
                path == root.trim_end_matches('/')
                    || archon_write_plan::lexical_path::under_root(&path, &root).is_some()
            });
    }
    #[cfg(not(windows))]
    [
        "/tmp",
        "/private/tmp",
        "/var/folders",
        "/private/var/folders",
    ]
    .iter()
    .any(|dir| path.starts_with(dir))
}
