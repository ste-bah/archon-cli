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
//! A freeze (before any run) probes only in the hermetic scratch site: it
//! never executes authored scripts in the live repository. With no scratch
//! policy the freeze-time probe does not run, and the acceptance round's own
//! in-round repair catches a crash the first time the round runs the check.
//! A probe that cannot run at all proves nothing either way: the check is not
//! held back, and the reason is kept as a diagnostic the caller records.
//!
//! Batch O (A4, A5): a probe given a pre-implementation baseline
//! ([`HostProbe::with_baseline`]) also proves each check CAN fail: it runs
//! the check on the tree before any implementation (the freeze's own HEAD,
//! or the run's base commit for a repair made mid-run), in a hermetic copy
//! only -- the scratch site, or a temporary clone -- never the live tree. A
//! check that passes there cannot show its criterion false and goes back to
//! its author as a defect; one the host could not run there is probed
//! again, never published unproven (`workflow_acceptance_executability_baseline`).

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
    /// code. Ids that ran, failed on their own assertion, or could not be
    /// probed are absent.
    async fn script_defects(
        &self,
        contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
    ) -> BTreeMap<String, String>;

    /// Why any probe could not run, drained by the caller.
    fn take_diagnostics(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Where a probe runs its checks.
enum Site {
    /// The `[workflow.acceptance_execution]` hermetic scratch observation.
    Scratch(NativeBinding),
    /// The live target repository, as the acceptance stage runs checks when
    /// no policy is configured. Only an acceptance round, which is about to
    /// run the same checks there anyway, probes here.
    Direct,
    /// Nowhere: why the gate could not run.
    Unavailable(String),
}

/// The host's probe, at the acceptance stage's own execution site.
pub(crate) struct HostProbe {
    project: PathBuf,
    repository: PathBuf,
    site: Site,
    diagnostics: Mutex<Vec<String>>,
    /// The pre-implementation tree every probed check must fail on.
    baseline: Option<baseline::Baseline>,
}

#[path = "workflow_acceptance_executability_baseline.rs"]
mod baseline;
pub(crate) use baseline::Baseline;

/// Sets the scratch observation's cancel flag when the probe is dropped (a
/// pause or cancel of the round), so its teardown starts at once.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl HostProbe {
    /// At an acceptance round's own site: its scratch policy, else the live
    /// target repository the round runs its checks in.
    pub(crate) fn at(
        project: PathBuf,
        repository: PathBuf,
        binding: Option<NativeBinding>,
    ) -> Self {
        let site = binding.map_or(Site::Direct, Site::Scratch);
        Self {
            project,
            repository,
            site,
            diagnostics: Mutex::new(Vec::new()),
            baseline: None,
        }
    }

    /// Also prove every probed check fails on `baseline`, the tree before
    /// any implementation (see the module docs).
    pub(crate) fn with_baseline(mut self, baseline: Baseline) -> Self {
        self.baseline = Some(baseline);
        self
    }

    /// At freeze time, before any run: only the hermetic scratch site. A
    /// freeze never executes authored scripts in the live repository; with no
    /// policy the checks are not probed here, and the acceptance round's own
    /// in-round repair catches a crash when it first runs them.
    pub(crate) fn for_task_set(project: &std::path::Path, tasks_root: &std::path::Path) -> Self {
        let site = match crate::command::acceptance_scratch_policy::capture(project, tasks_root) {
            Ok(Some(binding)) => Site::Scratch(binding),
            Ok(None) => Site::Unavailable(
                "no [workflow.acceptance_execution] is configured, and a freeze runs authored checks only in that hermetic scratch site, never in the live repository".into(),
            ),
            Err(error) => Site::Unavailable(format!(
                "the [workflow.acceptance_execution] policy could not be captured ({error})"
            )),
        };
        let repository = match &site {
            Site::Scratch(binding) => binding.policy.repository.clone(),
            Site::Direct | Site::Unavailable(_) => project.to_path_buf(),
        };
        Self {
            project: project.to_path_buf(),
            repository,
            site,
            diagnostics: Mutex::new(Vec::new()),
            baseline: None,
        }
    }

    fn note(&self, text: String) {
        self.diagnostics
            .lock()
            .expect("diagnostics lock")
            .push(text);
    }

    async fn run(
        &self,
        contract: &AcceptanceContract,
        digest: &str,
        refs: &[FrozenCommandRef],
    ) -> Vec<CheckResult> {
        let ids = || {
            refs.iter()
                .map(|r| r.acceptance_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        match &self.site {
            Site::Unavailable(reason) => {
                self.note(format!(
                    "executability probe not run: {reason}; check(s) {} were not executed before publication",
                    ids()
                ));
                Vec::new()
            }
            Site::Scratch(binding) => {
                let cancel = CancelOnDrop(Arc::new(AtomicBool::new(false)));
                let observed = tokio::spawn(observe(
                    binding.clone(),
                    None,
                    contract.clone(),
                    digest.to_string(),
                    refs.to_vec(),
                    cancel.0.clone(),
                ))
                .await
                .map_err(|error| anyhow::anyhow!("scratch probe task failed: {error}"))
                .and_then(|observed| observed);
                observed.unwrap_or_else(|error| {
                    self.note(format!(
                        "executability probe could not run in scratch ({error:#}); check(s) {} were not held back",
                        ids()
                    ));
                    Vec::new()
                })
            }
            Site::Direct => {
                let site = DirectSite {
                    repository: self.repository.clone(),
                    project: self.project.clone(),
                    environment: archon_tools::bash::host_env().into_iter().collect(),
                    timeout_secs: DIRECT_DEFAULT_TIMEOUT_SECS,
                    output_bytes: DIRECT_DEFAULT_OUTPUT_BYTES,
                };
                let cancel = Arc::new(AtomicBool::new(false));
                let mut results = Vec::new();
                for reference in refs {
                    match run_check_direct(&site, contract, digest, reference, cancel.clone()).await
                    {
                        Ok(result) => results.push(result),
                        Err(error) => self.note(format!(
                            "executability probe of '{}' could not run ({error}); it was not held back",
                            reference.acceptance_id
                        )),
                    }
                }
                results
            }
        }
    }
}

/// The scratch observation the acceptance stage's guardian performs, run
/// in-process over the unpublished candidate contract, at the target
/// repository's HEAD, under the same repository lease. Its own task, so a
/// synchronous copy phase never blocks the caller's run-control race; its
/// evidence is removed afterwards (a crash reaches the author as a finding).
async fn observe(
    binding: NativeBinding,
    commit: Option<String>,
    contract: AcceptanceContract,
    digest: String,
    refs: Vec<FrozenCommandRef>,
    cancel: Arc<AtomicBool>,
) -> anyhow::Result<Vec<CheckResult>> {
    let identity = binding.policy.repository.canonicalize()?;
    let _lease = crate::command::acceptance_scratch_guardian::acquire_lease(
        &std::env::temp_dir().join("archon-native-observer-locks"),
        &identity.to_string_lossy(),
    )?;
    let head = match commit {
        Some(commit) => commit,
        None => git_head(&binding.policy.repository)
            .ok_or_else(|| anyhow::anyhow!("cannot read the repository HEAD"))?,
    };
    let evidence = binding
        .policy
        .scratch_parent
        .join(format!("acceptance-probe-{}", uuid::Uuid::new_v4()));
    let observed = observe_commands_cancellable(
        &binding.policy,
        &head,
        &contract,
        &digest,
        &refs,
        &evidence,
        cancel,
    )
    .await;
    let _ = std::fs::remove_dir_all(&evidence);
    let observed = observed?;
    if !observed.operational_errors.is_empty()
        || !observed.teardown_verified
        || !observed.live_roots_unchanged
    {
        anyhow::bail!(
            "scratch observation void: {}",
            observed.operational_errors.join("; ")
        );
    }
    Ok(observed.checks)
}

fn git_head(repository: &std::path::Path) -> Option<String> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|head| head.len() == 40 && head.bytes().all(|b| b.is_ascii_hexdigit()))
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

#[async_trait]
impl ExecutabilityProbe for HostProbe {
    async fn script_defects(
        &self,
        contract: &AcceptanceContract,
        ids: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        // Only an accepted check resolves for execution; the candidate
        // contract is never written, so its digest is its own identity.
        let digest = match serde_json::to_vec(contract) {
            Ok(bytes) => content_digest(&bytes),
            Err(error) => {
                self.note(format!(
                    "executability probe could not encode the contract: {error}"
                ));
                return BTreeMap::new();
            }
        };
        let refs: Vec<FrozenCommandRef> = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .filter(|entry| ids.contains(&entry.id))
            .filter(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
            .filter_map(|entry| {
                let (kind, text) = executed_text(entry)?;
                Some(FrozenCommandRef {
                    acceptance_id: entry.id.clone(),
                    kind,
                    chain_digest: digest.clone(),
                    command_digest: content_digest(text.as_bytes()),
                })
            })
            .collect();
        if refs.is_empty() {
            return BTreeMap::new();
        }
        let results = self.run(contract, &digest, &refs).await;
        for result in &results {
            if let Some(error) = &result.operational_error {
                self.note(format!(
                    "executability probe of '{}' did not complete ({error}); it was not held back",
                    result.acceptance_id
                ));
            }
        }
        let mut findings = crash_findings(contract, &results);
        // A4/A5: what did not crash must also be able to fail.
        if let Some(baseline) = &self.baseline {
            // A site that probed nothing now (no scratch policy at a freeze)
            // still proves on the baseline's own hermetic copy.
            let probed = !matches!(self.site, Site::Unavailable(_));
            let sound: Vec<FrozenCommandRef> = (refs.iter())
                .filter(|reference| !findings.contains_key(&reference.acceptance_id))
                .filter(|reference| {
                    !probed
                        || results.iter().any(|result| {
                            result.acceptance_id == reference.acceptance_id
                                && result.operational_error.is_none()
                        })
                })
                .cloned()
                .collect();
            // The scratch site already observed HEAD: when HEAD is the
            // baseline (a freeze), its verdicts are the baseline's.
            let same_tree = match &self.site {
                Site::Scratch(binding) => {
                    git_head(&binding.policy.repository).as_deref()
                        == Some(baseline.commit.as_str())
                }
                Site::Direct | Site::Unavailable(_) => false,
            };
            let known = same_tree.then_some(results.as_slice());
            findings.extend(
                baseline::cannot_fail_findings(self, baseline, contract, &digest, &sound, known)
                    .await,
            );
        }
        findings
    }

    fn take_diagnostics(&self) -> Vec<String> {
        std::mem::take(&mut *self.diagnostics.lock().expect("diagnostics lock"))
    }
}

// The probe executes checks through the POSIX process-group runner.
#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_tests.rs"]
pub(crate) mod tests;
