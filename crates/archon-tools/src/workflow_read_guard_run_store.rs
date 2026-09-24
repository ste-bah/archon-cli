//! Refusing an agent write into the run's own bookkeeping directory.
//!
//! # The gap this closes
//!
//! A run keeps its records in a directory of its own: the per-branch result
//! envelopes, the stage inputs and outputs, the per-call coordination files,
//! the event log and the checkpoint. The host writes every one of them, the
//! host reads every one of them, and each is parsed on the input path of the
//! stage that follows — so a foreign file dropped among them is not one bad
//! file, it is a parse failure every later stage inherits. Live, a write
//! branch hand-authored a record of its own into the per-branch results
//! directory; the store read the directory as its own, failed on the foreign
//! file, and ninety-eight consecutive stages died in microseconds.
//!
//! The store was made to skip files it did not write itself, so that shape of
//! failure is gone. This module answers the question underneath it: why could
//! a branch write there at all. Two things said it could. The run directory
//! was advertised to agents as a writable artifact root
//! (`project_artifacts::artifact_roots_for_run`, which now advertises only the
//! artifact subdirectory), and nothing refused the write when it came — the
//! declared-target rule deliberately does not judge a path outside the branch
//! worktree, because writing to an artifact root outside it is legitimate.
//!
//! # The rule
//!
//! A write whose path resolves into the run STORE — the directory holding
//! this run and every run kept beside it — is refused, with two exceptions,
//! both inside the current run:
//!
//! 1. the run's artifact subdirectory, which is where a deliverable the run
//!    itself holds belongs, and
//! 2. the branch's own worktree, which the host places under the run
//!    directory. Refusing that would take away the agent's workspace, so it
//!    is exempted both by the working root the call was given and by the
//!    directories the host plants worktrees in, and a miss on either side
//!    would be the worse failure.
//!
//! Everything else is host bookkeeping — including every earlier run, which
//! belongs to no live call at all. The refusal names both places the agent
//! may write instead.
//!
//! # The read side
//!
//! [`RunStoreScope::holds_host_records`] answers the same question for the
//! recursive file walks, which is why the boundary is the store and not this
//! one run: the cost of walking it is the ACCUMULATED history, and an agent
//! that searched it walked tens of gigabytes of finished runs for 47 minutes
//! before it was killed by hand. One predicate serves both directions so the
//! two cannot drift.
//!
//! The judgement is on the RESOLVED path — `..` folded, the working root
//! prepended to a relative path — so a spelling cannot walk into the store
//! that a direct path would be refused for. Like the neighbouring rules this
//! covers the tools whose input names the file (`Write`, `Edit`, …) and a
//! shell write whose target the command spells out; a shell shape the lexer
//! cannot parse is not judged here, and the store's own indifference to
//! foreign files is what makes that acceptable.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::shell_writes::write_targets;

tokio::task_local! { static RUN_STORE: RunStoreScope; }

/// Directory names under a run root that hold agent-writable trees.
///
/// Relative to the run root, `/`-separated. `artifacts` is the run's own
/// deliverable area; the rest are where the host plants a branch worktree, and
/// are listed by the engine that plants them rather than inferred, so a new
/// layout has to be added here deliberately.
const AGENT_WRITABLE_SUBTREES: &[&str] = &["artifacts", "v2/worktrees", "wc/worktrees"];

/// The run store a tool path is judged against, and what stays writable
/// inside it.
#[derive(Debug, Clone, Default)]
pub struct RunStoreScope {
    /// The run STORE as given and canonicalised (`/var` against
    /// `/private/var` on macOS) — the directory the run directory sits in, so
    /// that runs finished long ago are covered by the same judgement. Empty
    /// when no run directory was known, which makes the whole scope inert.
    roots: Vec<PathBuf>,
    /// Absolute directories under a run root a write is still allowed into.
    exempt: Vec<PathBuf>,
    /// The run's artifact directory, for the refusal text.
    artifacts: Option<PathBuf>,
    /// Where the call runs, for the refusal text.
    working_root: Option<PathBuf>,
}

impl RunStoreScope {
    /// `run_root` is the run's own directory and `store_root` the directory
    /// every run of this project is kept in; `working_root` is the absolute
    /// directory the call runs in — the branch worktree for a write branch,
    /// which is exempt wherever the host placed it. Any may be absent:
    /// without a run root the scope is inert.
    ///
    /// `store_root` is a separate argument rather than `run_root.parent()`
    /// because only the caller knows whether the parent IS the store. Taking
    /// the parent here would be an assumption about a layout this crate does
    /// not own, and getting it wrong widens the boundary over an unrelated
    /// directory — refusing writes and pruning walks across a tree that has
    /// nothing to do with any run. Absent, the boundary is this run alone,
    /// which is the safe way to be wrong.
    pub fn new(
        run_root: Option<&str>,
        store_root: Option<&str>,
        working_root: Option<&str>,
    ) -> Self {
        let runs = spellings(run_root);
        if runs.is_empty() {
            return Self::default();
        }
        // Every run kept beside this one is host bookkeeping too, and it is
        // their accumulation that makes a walk of this tree expensive.
        let mut roots = spellings(store_root);
        for run in &runs {
            push_unique(&mut roots, run.clone());
        }
        let mut exempt = Vec::new();
        for run in &runs {
            for subtree in AGENT_WRITABLE_SUBTREES {
                push_unique(
                    &mut exempt,
                    run.join(subtree.replace('/', std::path::MAIN_SEPARATOR_STR)),
                );
            }
        }
        for spelling in spellings(working_root) {
            push_unique(&mut exempt, spelling);
        }
        Self {
            artifacts: Some(runs[0].join("artifacts")),
            working_root: spellings(working_root).into_iter().next(),
            roots,
            exempt,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// The refusal for `name` called with `input`, or `None` when the call
    /// writes nothing, names nothing this can read, or writes somewhere other
    /// than the run's own bookkeeping.
    pub(super) fn refusal(&self, name: &str, input: &Value) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        if name == "Bash" {
            let command = input.get("command").and_then(Value::as_str)?;
            return write_targets(command)
                .into_iter()
                .find_map(|write| self.judge(&write.path, Some(&write.head)));
        }
        if !mutates_a_file(name) {
            return None;
        }
        let target = ["file_path", "path"]
            .iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))?;
        self.judge(target.trim(), None)
    }

    /// Whether `path` is inside the run store and is NOT one of the two trees
    /// the current run's agent owns there.
    ///
    /// The write rule and the walk exclusion are the same boundary asked in
    /// two directions — may I create this file, and should I descend this
    /// directory — so they answer from one predicate rather than two lists
    /// that can drift. `path` is taken as absolute; a relative one is judged
    /// against the call's working root, which is itself exempt, so a caller
    /// with no root to anchor it gets `false`.
    pub fn holds_host_records(&self, path: &Path) -> bool {
        if self.is_empty() {
            return false;
        }
        let Some(resolved) = self.resolve(&path.to_string_lossy()) else {
            return false;
        };
        if self
            .exempt
            .iter()
            .any(|dir| resolved == *dir || resolved.starts_with(dir))
        {
            return false;
        }
        self.roots
            .iter()
            .any(|root| resolved == *root || resolved.starts_with(root))
    }

    /// Whether a recursive walk should stop at the directory `dir` rather
    /// than descend it.
    ///
    /// Not the same question as [`Self::holds_host_records`], and the
    /// difference is the whole reason this exists: the two trees the agent
    /// owns sit INSIDE the store, so the directories on the way down to them
    /// hold host records and must still be entered. A walk that pruned them
    /// would never reach the run's artifact area or the branch's own
    /// worktree, which is a walk that cannot see the agent's own files.
    ///
    /// So a directory is pruned only when nothing exempt lies beneath it.
    /// What that leaves entered is a corridor a few entries wide; what it
    /// prunes is every finished run and every record tree, which is all of
    /// the volume.
    pub fn prunes_walk(&self, dir: &Path) -> bool {
        if !self.holds_host_records(dir) {
            return false;
        }
        let Some(resolved) = self.resolve(&dir.to_string_lossy()) else {
            return false;
        };
        !self.exempt.iter().any(|tree| tree.starts_with(&resolved))
    }

    fn judge(&self, named: &str, head: Option<&str>) -> Option<String> {
        self.holds_host_records(Path::new(named))
            .then(|| self.refusal_text(named, head))
    }

    /// The absolute, lexically normalised path `named` refers to, or `None`
    /// when it cannot be placed — a relative path with no working root to
    /// anchor it, or one that climbs above the filesystem root.
    fn resolve(&self, named: &str) -> Option<PathBuf> {
        let path = Path::new(named);
        let anchored = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.working_root.as_ref()?.join(path)
        };
        normalise(&anchored)
    }

    fn refusal_text(&self, named: &str, head: Option<&str>) -> String {
        let subject = match head {
            Some(head) => format!("`{head}` writes {named}, which"),
            None => format!("Error: {named}"),
        };
        let artifacts = self
            .artifacts
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let workspace = match &self.working_root {
            Some(root) => format!(
                " Code and scratch files belong in your own workspace, {}.",
                root.display()
            ),
            None => String::new(),
        };
        format!(
            "{subject} is inside the run's own record directory. Those files are written and \
             read by the host alone — result envelopes, stage records, coordination state — and \
             a file of yours among them is parsed as one of them on the input path of every \
             later stage. The same holds for every earlier run kept beside this one. Write a \
             deliverable to {artifacts} instead, and report it in your envelope as an \
             artifact.{workspace} Never author a host record yourself: report what you would \
             have recorded in the envelope you return.",
        )
    }
}

/// A path as given and as canonicalised, both kept: a root reached through a
/// symlinked prefix must be recognised under either spelling, and a path that
/// does not exist yet cannot be canonicalised at all.
fn spellings(path: Option<&str>) -> Vec<PathBuf> {
    let Some(path) = path.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    let given = PathBuf::from(path);
    let canonical = std::fs::canonicalize(&given).unwrap_or_else(|_| given.clone());
    let mut out = Vec::new();
    push_unique(&mut out, given);
    push_unique(&mut out, canonical);
    out
}

fn push_unique(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.contains(&candidate) {
        paths.push(candidate);
    }
}

/// Lexical normalisation: `.` dropped, `..` folded; `None` when it climbs
/// above the filesystem root.
fn normalise(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(part) => out.push(part),
        }
    }
    Some(out)
}

/// The tools whose input names the file they will change; the staged
/// large-edit mutations carry only an `edit_id`, judged at `LargeEditBegin`.
fn mutates_a_file(name: &str) -> bool {
    matches!(
        name,
        "Write" | "Edit" | "MultiEdit" | "ApplyPatch" | "NotebookEdit" | "LargeEditBegin"
    )
}

/// Scope the run directory for the guard built inside `work`.
pub async fn scope_run_store<T>(
    scope: RunStoreScope,
    work: impl std::future::Future<Output = T>,
) -> T {
    RUN_STORE.scope(scope, work).await
}

/// The scope in effect, if any, for a caller outside this module: the tool
/// context copies it at construction so the file walks can prune the run
/// directory the same way the guard refuses writes into it.
pub fn current_run_store() -> Option<RunStoreScope> {
    current()
}

/// The scope in effect, if any: read once at guard construction.
pub(super) fn current() -> Option<RunStoreScope> {
    RUN_STORE
        .try_with(Clone::clone)
        .ok()
        .filter(|scope| !scope.is_empty())
}

#[cfg(test)]
#[path = "workflow_read_guard_run_store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_read_guard_run_store_walk_tests.rs"]
mod walk_tests;
