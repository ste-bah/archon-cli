//! Executability gate for authored acceptance checks.
//!
//! The judge weighs whether a check PROVES its criterion; it never runs it.
//! A check whose own script crashes (a helper called with the wrong
//! arguments, a syntax error) is judged accepted, published, and then fails
//! every acceptance round without ever asserting anything — and that failure
//! used to be routed to the implementing tasks, which cannot edit the
//! contract. So before an authored or re-authored check is published, the
//! host runs it once, at the site the acceptance stage would run it (the
//! `[workflow.acceptance_execution]` scratch observation when configured,
//! otherwise the live target repository), against the current tree, and
//! classifies the outcome with
//! [`archon_workflow::acceptance_check_crash::classify_check_run`]:
//!
//! * passed, or failed with the check's own assertion signal: publishable —
//!   the product may genuinely be failing;
//! * crashed in its own code: a finding for the author, like a judge
//!   refutation, and the bounded re-author loop runs again.
//!
//! A freeze (before any run) never executes authored scripts in the live
//! repository: it probes in the hermetic scratch site when one is
//! configured, and otherwise in the probe's own hermetic copy of the tree
//! (`workflow_acceptance_executability_hermetic`), which also serves a
//! project that lives outside its repository. A configured policy that
//! cannot be captured runs nothing at all. A check whose text names a live
//! root by its absolute path is refused before it runs: a copy cannot keep
//! it off the live tree.
//!
//! Batch O (A4, A5): a probe given a pre-implementation baseline
//! ([`HostProbe::with_baseline`]; a freeze-time probe carries the task
//! set's own) also proves each check CAN fail: it runs the check on the tree
//! before any implementation, in a hermetic copy only -- never the live
//! tree. A check that passes there must fail once the data it names is
//! moved aside (`workflow_acceptance_executability_mutation`), or it goes
//! back to its author as a defect. A probe given the tree an original check
//! ran on ([`HostProbe::with_failed_tree`], or the originals it observes
//! itself through [`ExecutabilityProbe::hold_originals`]) also holds a
//! repair to that verdict. What the host could not run, after repairing its
//! own environment, is never an author finding: it is UNPROVEN, the host's
//! ([`ExecutabilityProbe::take_unproven`], [`HostUnproven`]), and never
//! published.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use archon_workflow::acceptance_check_crash::{CheckRunClass, classify_check_run};
use archon_workflow::acceptance_scratch::{
    CheckResult, DIRECT_DEFAULT_OUTPUT_BYTES, DIRECT_DEFAULT_TIMEOUT_SECS, DirectSite,
    observe_commands_cancellable, run_check_direct,
};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, AcceptanceCriterion, JudgeDecision, content_digest,
};
use async_trait::async_trait;

use crate::command::acceptance_scratch_policy::NativeBinding;

/// Bytes of a crashed check's stderr shown to its author.
const FINDING_TAIL_BYTES: usize = 3000;

/// Runs accepted checks once and reports which crashed in their own code.
#[async_trait]
pub(crate) trait ExecutabilityProbe: Send + Sync {
    /// Author findings keyed by id, for each of `ids` that crashed in its own
    /// code (or cannot be shown able to fail). Ids that ran, failed on their
    /// own assertion, or could not be probed are absent.
    async fn script_defects(
        &self,
        contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
    ) -> BTreeMap<String, String>;

    /// As [`Self::script_defects`] for `ids` as frozen, recording how each
    /// fared on the probe's site as the original every repair of it is held
    /// to (A5).
    async fn hold_originals(
        &self,
        contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        self.script_defects(contract, ids).await
    }

    /// Why any probe could not run, drained by the caller.
    fn take_diagnostics(&self) -> Vec<String> {
        Vec::new()
    }

    /// Checks the host could not run even after repairing its environment,
    /// by id: never published, never the author's. Drained by the caller.
    fn take_unproven(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
}

/// The host could not prove these checks: an operational failure, routed to
/// the host (a round's operational error, retried; a freeze that refuses),
/// never to the check's author.
#[derive(Debug)]
pub(crate) struct HostUnproven(pub(crate) BTreeMap<String, String>);

/// How every [`HostUnproven`] failure starts, so a caller that only holds
/// its text can route it to the host.
pub(crate) const HOST_UNPROVEN: &str = "operational: the host could not prove";

impl std::fmt::Display for HostUnproven {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{HOST_UNPROVEN} {} check(s) after repairing its own environment, so nothing was published and no author was asked to change them: {}",
            self.0.len(),
            (self.0.iter())
                .map(|(id, why)| format!("{id}: {why}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}

impl std::error::Error for HostUnproven {}

/// Where a probe runs its checks.
enum Site {
    /// The `[workflow.acceptance_execution]` hermetic scratch observation.
    Scratch(Box<NativeBinding>),
    /// The live target repository, as the acceptance stage runs checks when
    /// no policy is configured. Only an acceptance round, which is about to
    /// run the same checks there anyway, probes here.
    Direct,
    /// The probe's own hermetic copy of the repository's HEAD and the
    /// project's data: a freeze with no scratch policy.
    Hermetic,
    /// A configured policy that could not be captured: nothing runs.
    Unavailable(String),
}

/// The host's probe, at the acceptance stage's own execution site.
pub(crate) struct HostProbe {
    project: PathBuf,
    repository: PathBuf,
    site: Site,
    diagnostics: Mutex<Vec<String>>,
    /// What the host could not run, after its environment repairs.
    unproven: Mutex<BTreeMap<String, String>>,
    /// The pre-implementation tree every probed check must fail on.
    baseline: Option<baseline::Baseline>,
    /// The tree the checks being repaired ran on, and how each fared there.
    failed_tree: Mutex<Option<baseline::FailedTree>>,
    /// Where the probe's own hermetic copies are made.
    copy_parent: PathBuf,
    /// Reuse a verdict already observed at the same commit in this process:
    /// only a freeze, whose trees do not move under it, sets it.
    memo: bool,
    /// Hermetic runs that fail before running anything, for tests.
    #[cfg(test)]
    injected_failures: std::sync::atomic::AtomicUsize,
    /// Hermetic copies made, for tests.
    #[cfg(test)]
    copies_made: std::sync::atomic::AtomicUsize,
}

#[path = "workflow_acceptance_executability_baseline.rs"]
mod baseline;
pub(crate) use baseline::{Baseline, FailedTree, PLACEHOLDER_REASON, originals};
#[cfg(test)]
pub(crate) use baseline::{Original, is_placeholder};
use sites::git_head;
#[path = "workflow_acceptance_executability_hermetic.rs"]
mod hermetic;
#[path = "workflow_acceptance_executability_mutation.rs"]
mod mutation;
#[path = "workflow_acceptance_executability_probe.rs"]
mod probe;
#[path = "workflow_acceptance_executability_prove.rs"]
mod prove;
#[path = "workflow_acceptance_executability_repairs.rs"]
mod repairs;
#[path = "workflow_acceptance_executability_sites.rs"]
mod sites;
pub(crate) use mutation::CANNOT_FAIL;

/// Sets the scratch observation's cancel flag when the probe is dropped (a
/// pause or cancel of the round), so its teardown starts at once.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl HostProbe {
    fn new(project: PathBuf, repository: PathBuf, site: Site) -> Self {
        Self {
            project,
            repository,
            site,
            diagnostics: Mutex::new(Vec::new()),
            unproven: Mutex::new(BTreeMap::new()),
            baseline: None,
            failed_tree: Mutex::new(None),
            copy_parent: std::env::temp_dir(),
            memo: false,
            #[cfg(test)]
            injected_failures: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            copies_made: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Fail the next `count` hermetic runs as the host's environment would.
    #[cfg(test)]
    pub(crate) fn with_injected_failures(self, count: usize) -> Self {
        (self.injected_failures).store(count, std::sync::atomic::Ordering::SeqCst);
        self
    }

    #[cfg(test)]
    fn take_injected_failure(&self) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        (self.injected_failures)
            .fetch_update(SeqCst, SeqCst, |left| left.checked_sub(1))
            .is_ok()
    }

    /// At an acceptance round's own site: its scratch policy, else the live
    /// target repository the round runs its checks in.
    pub(crate) fn at(
        project: PathBuf,
        repository: PathBuf,
        binding: Option<NativeBinding>,
    ) -> Self {
        let site = binding.map_or(Site::Direct, |binding| Site::Scratch(Box::new(binding)));
        Self::new(project, repository, site)
    }

    /// Also prove every probed check fails on `baseline`, the tree before
    /// any implementation (see the module docs).
    pub(crate) fn with_baseline(mut self, baseline: Baseline) -> Self {
        self.baseline = Some(baseline);
        self
    }

    /// Also hold every probed check to its original's verdict on the tree
    /// that original ran on (A5, `FailedTree`).
    pub(crate) fn with_failed_tree(self, tree: FailedTree) -> Self {
        *self.failed_tree.lock().expect("failed tree lock") = Some(tree);
        self
    }

    /// Make the probe's own hermetic copies under `parent`.
    #[cfg(test)]
    pub(crate) fn with_copy_parent(mut self, parent: PathBuf) -> Self {
        self.copy_parent = parent;
        self
    }

    /// At freeze time, before any run: the hermetic scratch site when one is
    /// configured (building warm from a per-repository cache), otherwise the
    /// probe's own hermetic copy -- never the live repository. A configured
    /// policy that cannot be captured runs nothing. The probe carries the
    /// task set's pre-implementation baseline (`Baseline::for_task_set`).
    pub(crate) fn for_task_set(project: &std::path::Path, tasks_root: &std::path::Path) -> Self {
        let repository = baseline::task_set_repository(project, tasks_root);
        let site = match crate::command::acceptance_scratch_policy::capture(project, tasks_root) {
            Ok(Some(binding)) => {
                let key = content_digest(binding.policy.repository.to_string_lossy().as_bytes());
                Site::Scratch(Box::new(
                    binding.with_run_build_cache(&format!("acceptance-probe-{}", &key[..12])),
                ))
            }
            Ok(None) => Site::Hermetic,
            Err(error) => Site::Unavailable(format!(
                "the [workflow.acceptance_execution] policy could not be captured ({error}); nothing is run until it is repaired"
            )),
        };
        let repository = match &site {
            Site::Scratch(binding) => binding.policy.repository.clone(),
            Site::Direct | Site::Hermetic | Site::Unavailable(_) => repository,
        };
        let mut probe = Self::new(project.to_path_buf(), repository, site);
        probe.memo = true;
        probe.baseline = Baseline::for_task_set(&probe.repository, tasks_root);
        if let Site::Scratch(binding) = &probe.site
            && let Some(cache) = &binding.policy.build_cache
        {
            probe.note(format!(
                "the freeze probe builds warm from the scratch build cache {}",
                cache.display()
            ));
        }
        if probe.baseline.is_none() {
            probe.note(format!(
                "pre-implementation probe not run: {} is not a git checkout with a commit, so the checks are not proven able to fail",
                probe.repository.display()
            ));
        }
        probe
    }

    fn note(&self, text: String) {
        self.diagnostics
            .lock()
            .expect("diagnostics lock")
            .push(text);
    }

    /// Record that the host could not prove `id`.
    fn unproven(&self, id: &str, why: String) {
        self.unproven
            .lock()
            .expect("unproven lock")
            .insert(id.to_string(), why);
    }
}

/// The check text the host executes for `entry`, if it has one: a command,
/// or a floor's non-blank verifier. A declarative floor has no script.
pub(crate) fn executed_text(entry: &AcceptanceCriterion) -> Option<(AcceptanceCommandKind, &str)> {
    match &entry.check {
        AcceptanceCheck::Command { command, .. } => {
            Some((AcceptanceCommandKind::Command, command.as_str()))
        }
        AcceptanceCheck::Floor { contract } => contract
            .typed_verifier_command
            .as_deref()
            .filter(|command| !command.trim().is_empty())
            .map(|command| (AcceptanceCommandKind::NestedVerifier, command)),
    }
}

/// Author findings for every result in `results` whose check crashed in its
/// own code.
pub(crate) fn crash_findings<'a>(
    contract: &AcceptanceContract,
    results: impl IntoIterator<Item = &'a CheckResult>,
) -> BTreeMap<String, String> {
    results
        .into_iter()
        .filter_map(|result| {
            let entry = contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .find(|entry| entry.id == result.acceptance_id)?;
            let (_, text) = executed_text(entry)?;
            match classify_check_run(text, result) {
                CheckRunClass::ScriptDefect(defect) => Some((
                    entry.id.clone(),
                    defect.finding(&entry.id, &stderr_tail(&result.stderr)),
                )),
                CheckRunClass::Passed | CheckRunClass::Failed => None,
            }
        })
        .collect()
}

/// The crash's stderr as the repairing agent sees it: its end and every line
/// stating the failure, bounded.
fn stderr_tail(bytes: &[u8]) -> String {
    archon_workflow::failure_evidence::failure_evidence(bytes, FINDING_TAIL_BYTES)
}

/// A reference per accepted, script-bearing entry of `contract` in `ids`,
/// under `digest`.
fn refs_for(
    contract: &AcceptanceContract,
    digest: &str,
    ids: &BTreeSet<String>,
) -> Vec<FrozenCommandRef> {
    (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .filter(|entry| ids.contains(&entry.id))
        .filter(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
        .filter_map(|entry| {
            let (kind, text) = executed_text(entry)?;
            Some(FrozenCommandRef {
                acceptance_id: entry.id.clone(),
                kind,
                chain_digest: digest.to_string(),
                command_digest: content_digest(text.as_bytes()),
            })
        })
        .collect()
}

/// A candidate contract's own identity: it is never written, so its
/// digest is its content's.
fn contract_digest(contract: &AcceptanceContract) -> Result<String, String> {
    serde_json::to_vec(contract)
        .map(|bytes| content_digest(&bytes))
        .map_err(|error| error.to_string())
}

// The probe executes checks through the POSIX process-group runner.
#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_probe_tests.rs"]
mod probe_tests;
#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_probe_tests_b.rs"]
mod probe_tests_b;
#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_tests.rs"]
pub(crate) mod tests;
