//! A declared path the repository ignores is a project artifact, never a
//! repository-audit obligation (Issue-26).
//!
//! The write layer already treats such a deliverable as operator policy: the
//! patch sidecar archives it, the manifest records it as `skipped_ignored`, the
//! branch reports it "not committed; retained at <artifact>" and the outcome
//! is accepted (`worktree_branch_b::report_ignored_deliverables`). The audit
//! did not know this. Every declared path went into the audit's jurisdiction,
//! the sealed view — `git add -A` honours `.gitignore` — never held the file,
//! the assessor recorded it `absent / deliver`, the ledger opened an
//! obligation nothing could ever resolve, `reuse::eligible` refused the
//! accepted wave, and the task was re-dispatched on every resume. Live:
//! TASK-DL-001, `docs/trading-data-lake-gap-audit.md` under `.gitignore:67
//! /docs/*`, six re-runs of 7–30 minutes each.
//!
//! One predicate answers for every place the jurisdiction grows: `git
//! check-ignore`, which never reports a tracked file (a tracked file matched by
//! a pattern is still a repository deliverable) and reports nothing at all for
//! a root that is not a repository. Paths the audit reclaims are remembered on
//! the ledger so a later refresh does not read their absence as a silently
//! dropped declaration and so cache admission can leave them out of the
//! question it asks.
use std::collections::BTreeSet;
use std::path::Path;

use super::runtime::{AuditRuntime, AuditState};
use crate::write_coordinator::worktree_isolation::{run_git, run_git_with_stdin};
use crate::{WorkflowEventKind, WorkflowResult};

/// Whether the repository at `repo_root` ignores `path` (root-relative).
pub fn is_ignored(repo_root: &Path, path: &str) -> bool {
    run_git(&["check-ignore", "-q", "--", path], repo_root).is_ok()
}

/// The subset of `paths` the repository at `repo_root` ignores, in one
/// `git check-ignore --stdin` call. A non-zero exit means "none of them":
/// exit 1 is git's own "nothing ignored", anything else is not a repository.
pub fn ignored_among(repo_root: &Path, paths: &[String]) -> BTreeSet<String> {
    if paths.is_empty() {
        return BTreeSet::new();
    }
    let mut stdin = Vec::new();
    for path in paths {
        stdin.extend_from_slice(path.as_bytes());
        stdin.push(0);
    }
    let Ok(output) = run_git_with_stdin(&["check-ignore", "--stdin", "-z"], repo_root, &stdin)
    else {
        return BTreeSet::new();
    };
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| String::from_utf8(entry.to_vec()).ok())
        .filter(|entry| paths.contains(entry))
        .collect()
}

/// `paths` less those the repository at `repo_root` ignores.
pub fn repository_deliverables(repo_root: &Path, paths: Vec<String>) -> Vec<String> {
    let ignored = ignored_among(repo_root, &paths);
    paths
        .into_iter()
        .filter(|path| !ignored.contains(path))
        .collect()
}

impl AuditState {
    /// Take `ignored` out of the audit's jurisdiction: out of `declared_paths`
    /// and `ledger.obligations`, into `ledger.ignored_paths`. History is
    /// append-only and waivers are the operator's; neither is touched. Returns
    /// the paths this changed anything for — declared or obligated until now,
    /// or reclaimed for the first time — so a wave re-declaring the same
    /// artifact on every dispatch is reported once, not every time.
    pub fn drop_ignored(&mut self, ignored: &BTreeSet<String>) -> Vec<String> {
        let mut dropped = Vec::new();
        for path in ignored {
            let declared = self.declared_paths.remove(path);
            let obligated = self.ledger.obligations.remove(path).is_some();
            let first = self.ledger.ignored_paths.insert(path.clone());
            if declared || obligated || first {
                dropped.push(path.clone());
            }
        }
        dropped
    }
}

impl AuditRuntime {
    /// Reclaim every declared or obligated path the repository at `repo_root`
    /// ignores, so a run whose state already carries such an obligation stops
    /// looping without operator surgery. Emits
    /// `repository_audit_ignored_paths_dropped` naming what was dropped.
    pub fn reclaim_ignored(&self, repo_root: &Path) -> WorkflowResult<Vec<String>> {
        let state = self.state()?;
        let known = state
            .declared_paths
            .iter()
            .chain(state.ledger.obligations.keys())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let ignored = ignored_among(repo_root, &known);
        if ignored.is_empty() {
            return Ok(Vec::new());
        }
        let dropped = self.update(|state| Ok(state.drop_ignored(&ignored)))?;
        self.report_dropped(repo_root, &dropped)?;
        Ok(dropped)
    }

    /// The `repository_audit_ignored_paths_dropped` event, when anything was.
    pub(super) fn report_dropped(
        &self,
        repo_root: &Path,
        dropped: &[String],
    ) -> WorkflowResult<()> {
        if dropped.is_empty() {
            return Ok(());
        }
        self.event(
            WorkflowEventKind::StageCompleted,
            serde_json::json!({
                "event": "repository_audit_ignored_paths_dropped",
                "checked_against": repo_root,
                "paths": dropped,
                "reason": "gitignored declared paths are project artifacts, not repository deliverables",
            }),
        )
    }
}
