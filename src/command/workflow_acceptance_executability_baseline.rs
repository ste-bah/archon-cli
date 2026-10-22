//! The trees a probed check is held to: it must be able to fail (A4, A5),
//! and a repair must keep its original's verdict (A5).
//!
//! The judge weighs a check; it never runs one, so a check that passes
//! whatever the implementation does was published and then "passed". Here a
//! probe given a [`Baseline`] runs each check that did not crash on the tree
//! BEFORE any implementation and requires it to fail there:
//!
//! - it failed there: sound;
//! - it passed there: it is run once more with the inputs it names removed
//!   (`workflow_acceptance_executability_mutation`); failing then, it is a
//!   regression guard for a criterion the tree already meets (a diagnostic),
//!   otherwise a finding for its author -- it cannot show its criterion
//!   false, so it proves nothing;
//! - the host could not run it there, even after repairing its own
//!   environment (`workflow_acceptance_executability_repairs`): UNPROVEN --
//!   the host's, never the author's, and never published.
//!
//! A probe given a [`FailedTree`] also runs each repaired check on the tree
//! the check it replaces ran on. The rule: a repair must not newly pass
//! where its original failed its OWN ASSERTION ([`Original::Failed`]) -- that
//! verdict was on the product, and a repair may fix how a check asserts its
//! criterion, never turn a failing product green without a product change.
//! Where the original's failure was a defect of the check itself
//! ([`Original::Defect`]: it crashed in its own code, the host could not run
//! it, or it was a contract defect that never ran) it gave no verdict on
//! that tree, so the repair's pass there is the first verdict, not a
//! weakening; it is held only to the baseline.
//!
//! Every run is hermetic, never the live tree: the scratch site's own
//! observation at the commit, or the probe's own copy of the repository at
//! it and of the project's data (`workflow_acceptance_executability_hermetic`),
//! which serves a project outside its repository as well as one inside it.

use std::path::{Path, PathBuf};

use super::repairs::{TreeRun, timed_out, tree_results};
use super::sites::{object_id, resolves};
use super::*;

/// The tree before any implementation, in the repository it belongs to.
#[derive(Debug, Clone)]
pub(crate) struct Baseline {
    pub(crate) commit: String,
    pub(crate) repository: PathBuf,
}

impl Baseline {
    /// The tree a freeze stands on: `repository`'s HEAD. `None` when it is
    /// not a git checkout.
    pub(crate) fn head_of(repository: &Path) -> Option<Self> {
        Some(Self {
            commit: git_head(repository)?,
            repository: repository.to_path_buf(),
        })
    }

    /// The tree a task set's checks are proven able to fail on (Issue 328:
    /// one baseline): the commit its frozen acceptance lock records as the
    /// tree its freeze proved on; else the commit its decomposition recorded
    /// (`repository.lock`), which stays the pre-implementation tree when a
    /// repair runs after work has landed; else `repository`'s HEAD.
    pub(crate) fn for_task_set(repository: &Path, tasks_root: &Path) -> Option<Self> {
        Self::recorded(repository, tasks_root)
            .or_else(|| Self::decomposed(repository, tasks_root))
            .or_else(|| Self::head_of(repository))
    }

    /// The commit the task set's frozen acceptance lock records as its
    /// freeze's baseline, when it names a commit of `repository`.
    pub(crate) fn recorded(repository: &Path, tasks_root: &Path) -> Option<Self> {
        let commit = recorded_commit(tasks_root).filter(|commit| usable(repository, commit))?;
        Some(Self {
            commit,
            repository: repository.to_path_buf(),
        })
    }

    /// The commit the task set's decomposition recorded, when it resolves.
    fn decomposed(repository: &Path, tasks_root: &Path) -> Option<Self> {
        let commit = archon_workflow::repository_record::read_repository_record(tasks_root)
            .ok()
            .flatten()
            .map(|record| record.base_commit)
            .filter(|commit| usable(repository, commit))?;
        Some(Self {
            commit,
            repository: repository.to_path_buf(),
        })
    }

    /// The tree an acceptance round proves its checks on: the one its
    /// task set's freeze recorded. Only a lock that records none (written
    /// before the record existed) falls back -- to the tree the freeze's own
    /// rule used (`repository.lock`), else the run's base `run_base`, else
    /// HEAD -- and then the note says which.
    pub(crate) fn for_round(
        repository: &Path,
        tasks_root: &Path,
        run_base: Option<&str>,
    ) -> (Option<Self>, Option<String>) {
        if let Some(recorded) = Self::recorded(repository, tasks_root) {
            return (Some(recorded), None);
        }
        let missing = match recorded_commit(tasks_root) {
            Some(commit) => format!(
                "the acceptance freeze recorded baseline {commit}, which is not a commit of {}",
                repository.display()
            ),
            None => "the acceptance freeze recorded no baseline commit".to_string(),
        };
        let (baseline, used) = match Self::decomposed(repository, tasks_root) {
            Some(baseline) => (Some(baseline), "the decomposition's recorded base commit"),
            None => match run_base.filter(|commit| usable(repository, commit)) {
                Some(commit) => (
                    Some(Self {
                        commit: commit.to_string(),
                        repository: repository.to_path_buf(),
                    }),
                    "the run's base commit",
                ),
                None => (Self::head_of(repository), "the repository's HEAD"),
            },
        };
        let note = match &baseline {
            Some(baseline) => format!(
                "{missing}; checks are proven able to fail on {used}, {}",
                baseline.commit
            ),
            None => format!("{missing}, and no other pre-implementation tree is known"),
        };
        (baseline, Some(note))
    }

    /// The commit a v3 acceptance round in `run_dir` proves its checks able
    /// to fail on ([`Self::for_round`], with the run's own base as its last
    /// fallback); a fallback is logged.
    pub(crate) fn round_commit(
        repository: &Path,
        tasks_root: &Path,
        run_dir: &Path,
    ) -> Option<String> {
        let run_base = archon_workflow::v2::acceptance_regression::run_base_commit(
            &archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2")),
        );
        let (baseline, note) = Self::for_round(repository, tasks_root, run_base.as_deref());
        if let Some(note) = note {
            tracing::warn!(task_root = %tasks_root.display(), "acceptance baseline: {note}");
        }
        baseline.map(|baseline| baseline.commit)
    }
}

/// Whether the recorded `commit` may stand as a baseline in `repository`:
/// a full object id that is a commit there, or -- when `repository` is no
/// git checkout to ask -- taken as recorded, so a proof already recorded
/// for that id is still found without git.
fn usable(repository: &Path, commit: &str) -> bool {
    object_id(commit) && (resolves(repository, commit) || git_head(repository).is_none())
}

/// The baseline commit `tasks_root`'s acceptance lock records, if any.
pub(crate) fn recorded_commit(tasks_root: &Path) -> Option<String> {
    let path = tasks_root.join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE);
    let bytes = std::fs::read(path).ok()?;
    let lock: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let commit = lock.get("baseline_commit")?.as_str()?.trim();
    (!commit.is_empty()).then(|| commit.to_string())
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ReuseAudit {
    pub(crate) reused: bool,
    pub(crate) why: String,
    pub(crate) evidence: Option<String>,
    pub(crate) input_key: Option<String>,
}

/// How the checks that failed on a probe's pre-implementation tree failed
/// there (Issue 275), and the environment they ran with.
#[derive(Debug, Clone)]
pub(crate) struct BaselineRuns {
    /// Per-check cache decision and original evidence reference.
    pub(crate) reuse: BTreeMap<String, ReuseAudit>,
    pub(crate) commit: String,
    pub(crate) failures: BTreeMap<String, super::hermetic::CheckEvidence>,
    /// The environment the site gave each check: what its output could echo.
    pub(crate) environment: BTreeMap<String, String>,
    /// Names the site's policy forwards from the host: secret by declaration.
    pub(crate) forwarded: Vec<String>,
}

impl HostProbe {
    /// Whether the latest check-copy observation reused durable evidence.
    pub(crate) fn reused_check(&self, id: &str) -> bool {
        self.reuse_audit
            .lock()
            .ok()
            .and_then(|audit| audit.get(id).map(|entry| entry.reused))
            .unwrap_or(false)
    }

    /// The per-check stable reuse keys observed by the latest probe run.
    pub(crate) fn take_reuse_audit(&self) -> BTreeMap<String, ReuseAudit> {
        std::mem::take(&mut *self.reuse_audit.lock().expect("reuse audit lock"))
    }

    fn failed_on_baseline(&self, result: &super::hermetic::CheckEvidence) {
        (self
            .baseline_failures
            .lock()
            .expect("baseline failures lock"))
        .insert(result.acceptance_id.clone(), result.clone());
    }

    /// The baseline evidence recorded so far, drained (Issue 275).
    pub(super) fn baseline_runs(&self) -> Option<BaselineRuns> {
        let failures = std::mem::take(
            &mut *self
                .baseline_failures
                .lock()
                .expect("baseline failures lock"),
        );
        let (environment, forwarded) = super::sites::site_environment(self);
        Some(BaselineRuns {
            commit: self.baseline.as_ref()?.commit.clone(),
            reuse: std::mem::take(&mut *self.reuse_audit.lock().expect("reuse audit lock")),
            failures,
            environment,
            forwarded,
        })
    }
}

/// How the check a repair replaces fared on the tree it ran on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Original {
    /// It failed its own assertion: a verdict on the product.
    Failed,
    /// It passed.
    Passed,
    /// It crashed in its own code, could not be run, or never ran: no
    /// verdict on that tree.
    Defect,
}

/// The tree the checks being repaired ran on. `commit` is its HEAD, `None`
/// when the probe's own site is that tree.
#[derive(Debug, Clone)]
pub(crate) struct FailedTree {
    pub(crate) commit: Option<String>,
    pub(crate) originals: BTreeMap<String, Original>,
}

/// The judge reason a host-created placeholder entry carries: a criterion
/// owed a check nobody has authored yet (its check is not an author's).
pub(crate) const PLACEHOLDER_REASON: &str = "no check has been authored for this criterion yet";

/// Whether `entry` is a host placeholder rather than an authored check: not
/// accepted, and carrying [`PLACEHOLDER_REASON`]. A placeholder is never a
/// strength baseline for its repair: only the can-fail probe applies.
pub(crate) fn is_placeholder(entry: &AcceptanceCriterion) -> bool {
    entry.judgment.verdict != JudgeDecision::Accepted
        && entry.judgment.reason.trim() == PLACEHOLDER_REASON
}

/// How each of `results` fared, by id (`originals_at`), decided on a
/// blocking thread: deciding it may list a tool, which must never hold an
/// async worker (Issue 333).
/// They are judged at `at`, the site they ran on (`HostProbe::check_site`),
/// never the host: a program the host has but that site lacks is no
/// verdict there, and a listing never sees the host's environment.
pub(crate) async fn originals(
    at: &super::verdict::Context,
    contract: &AcceptanceContract,
    results: Vec<CheckResult>,
) -> BTreeMap<String, Original> {
    let (at, contract) = (at.clone(), contract.clone());
    let ids: Vec<String> = results.iter().map(|r| r.acceptance_id.clone()).collect();
    tokio::task::spawn_blocking(move || originals_at(&contract, &results, &at))
        .await
        .unwrap_or_else(|_| ids.into_iter().map(|id| (id, Original::Defect)).collect())
}

/// How each of `results` fared, by id. A placeholder's result is never a
/// verdict of its own ([`Original::Defect`]).
#[cfg(test)]
pub(crate) fn originals_now<'a>(
    contract: &AcceptanceContract,
    results: impl IntoIterator<Item = &'a CheckResult>,
) -> BTreeMap<String, Original> {
    originals_at(
        contract,
        results,
        &super::verdict::Context::on_host_path(contract),
    )
}

fn originals_at<'a>(
    contract: &AcceptanceContract,
    results: impl IntoIterator<Item = &'a CheckResult>,
    at: &super::verdict::Context,
) -> BTreeMap<String, Original> {
    let results: Vec<&CheckResult> = results.into_iter().collect();
    let crashed = crash_findings(contract, results.iter().copied());
    results
        .into_iter()
        .map(|result| {
            let id = result.acceptance_id.clone();
            let placeholder = (contract.acceptance.iter())
                .chain(&contract.supplementary)
                .any(|entry| entry.id == id && is_placeholder(entry));
            // Issue 328: a run that gave no verdict (a program that could
            // not start, a tree that did not build) failed no assertion.
            let silent = silent::silent_failure(contract, result, at).is_some();
            let original = if placeholder
                || crashed.contains_key(&id)
                || result.operational_error.is_some()
                || silent
            {
                Original::Defect
            } else if passed(result) {
                Original::Passed
            } else {
                Original::Failed
            };
            (id, original)
        })
        .collect()
}

#[path = "workflow_acceptance_executability_baseline_findings.rs"]
mod findings;
pub(super) use findings::{cannot_fail_findings, newly_passing_findings, passed};
