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

use super::repairs::{TreeRun, tree_results};
use super::sites::resolves;
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

    /// The tree a task set's implementation starts from: the commit its
    /// decomposition recorded (`repository.lock`), which stays the
    /// pre-implementation tree when a repair runs after work has landed,
    /// else `repository`'s HEAD.
    pub(crate) fn for_task_set(repository: &Path, tasks_root: &Path) -> Option<Self> {
        let recorded = archon_workflow::repository_record::read_repository_record(tasks_root)
            .ok()
            .flatten()
            .map(|record| record.base_commit)
            .filter(|commit| resolves(repository, commit));
        match recorded {
            Some(commit) => Some(Self {
                commit,
                repository: repository.to_path_buf(),
            }),
            None => Self::head_of(repository),
        }
    }
}

/// The repository a task set was decomposed against, else the project.
pub(super) fn task_set_repository(project: &Path, tasks_root: &Path) -> PathBuf {
    archon_workflow::repository_record::read_repository_record(tasks_root)
        .ok()
        .flatten()
        .map(|record| PathBuf::from(record.repository_root))
        .filter(|root| root.is_dir())
        .unwrap_or_else(|| project.to_path_buf())
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

/// How each of `results` fared, by id. A placeholder's result is never a
/// verdict of its own ([`Original::Defect`]).
pub(crate) fn originals<'a>(
    contract: &AcceptanceContract,
    results: impl IntoIterator<Item = &'a CheckResult>,
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
            let original =
                if placeholder || crashed.contains_key(&id) || result.operational_error.is_some() {
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

/// A pass the acceptance stage would count: exit 0 with real work.
pub(super) fn passed(result: &CheckResult) -> bool {
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    result.operational_error.is_none()
        && result.exit_code == Some(0)
        && !archon_workflow::acceptance::output_reports_zero_work(&stdout, &stderr)
}

/// Author findings, keyed by id, for every check of `refs` that cannot be
/// shown to fail on the baseline or could not be run there (see the module
/// docs).
pub(super) async fn cannot_fail_findings(
    probe: &HostProbe,
    baseline: &Baseline,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
    known: Option<&[CheckResult]>,
) -> BTreeMap<String, String> {
    let mut findings = BTreeMap::new();
    if refs.is_empty() {
        return findings;
    }
    let short: String = baseline.commit.chars().take(12).collect();
    let run = tree_results(probe, baseline, contract, digest, refs, known, false).await;
    let mut passing = Vec::new();
    for reference in refs {
        let id = &reference.acceptance_id;
        let result = run.results.get(id);
        if let Some(reason) = run.unrun(id) {
            probe.unproven(
                id,
                format!(
                    "it could not be run on the pre-implementation tree at {short} ({reason}), so it is not proven able to fail"
                ),
            );
        } else if result.is_some_and(passed) {
            passing.push(id.clone());
        }
    }
    findings.extend(prove::prove(probe, baseline, contract, &passing).await);
    findings
}

/// Findings for every check of `refs` whose original failed its own
/// assertion on `tree` and which passes there (see the module docs).
pub(super) async fn newly_passing_findings(
    probe: &HostProbe,
    tree: &FailedTree,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
    known: Option<&[CheckResult]>,
) -> BTreeMap<String, String> {
    let held: Vec<FrozenCommandRef> = (refs.iter())
        .filter(|reference| tree.originals.get(&reference.acceptance_id) == Some(&Original::Failed))
        .cloned()
        .collect();
    let mut findings = BTreeMap::new();
    if held.is_empty() {
        return findings;
    }
    let run = match (known, &tree.commit) {
        (Some(known), _) => TreeRun::known(known, "it did not run at the probe's site"),
        (None, Some(commit)) => {
            let at = Baseline {
                commit: commit.clone(),
                repository: probe.repository.clone(),
            };
            tree_results(probe, &at, contract, digest, &held, None, false).await
        }
        (None, None) => TreeRun::known(&[], "the tree has no commit to copy"),
    };
    let short: String = (tree.commit.as_deref().unwrap_or("the round's tree").chars())
        .take(12)
        .collect();
    for reference in &held {
        let id = &reference.acceptance_id;
        let result = run.results.get(id);
        if let Some(reason) = run.unrun(id) {
            probe.unproven(
                id,
                format!(
                    "the repair could not be run on the tree its original failed on ({short}: {reason}), so it is not proven to keep that verdict"
                ),
            );
        } else if result.is_some_and(passed) {
            findings.insert(
                id.clone(),
                format!(
                    "check '{id}': the check it replaces failed its own assertion on the tree at {short}, and this repair passes there; a repair may fix how a check asserts its criterion, never turn a failing product green without a product change: keep every assertion the original made, so that it still fails on that tree"
                ),
            );
        }
    }
    findings
}
