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
//! A probe that cannot run at all (scratch unavailable, the repository lease
//! held) proves nothing either way: the check is not held back, and the
//! reason is kept as a diagnostic.

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

/// The host's probe, at the acceptance stage's own execution site.
pub(crate) struct HostProbe {
    project: PathBuf,
    repository: PathBuf,
    binding: Option<NativeBinding>,
    diagnostics: Mutex<Vec<String>>,
}

impl HostProbe {
    pub(crate) fn at(
        project: PathBuf,
        repository: PathBuf,
        binding: Option<NativeBinding>,
    ) -> Self {
        Self {
            project,
            repository,
            binding,
            diagnostics: Mutex::new(Vec::new()),
        }
    }

    /// The site a run of this task set would use: the configured scratch
    /// policy, else the repository the task set was decomposed against.
    pub(crate) fn for_task_set(
        project: &std::path::Path,
        tasks_root: &std::path::Path,
        repository_root: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let binding = crate::command::acceptance_scratch_policy::capture(project, tasks_root)?;
        let repository = binding.as_ref().map_or_else(
            || repository_root.to_path_buf(),
            |binding| binding.policy.repository.clone(),
        );
        Ok(Self::at(project.to_path_buf(), repository, binding))
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
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(binding) = &self.binding {
            return match self.observe(binding, contract, digest, refs, cancel).await {
                Ok(checks) => checks,
                Err(error) => {
                    self.note(format!(
                        "executability probe could not run in scratch ({error}); the check(s) {} were not held back",
                        refs.iter().map(|r| r.acceptance_id.as_str()).collect::<Vec<_>>().join(", ")
                    ));
                    Vec::new()
                }
            };
        }
        let site = DirectSite {
            repository: self.repository.clone(),
            project: self.project.clone(),
            environment: archon_tools::bash::host_env().into_iter().collect(),
            timeout_secs: DIRECT_DEFAULT_TIMEOUT_SECS,
            output_bytes: DIRECT_DEFAULT_OUTPUT_BYTES,
        };
        let mut results = Vec::new();
        for reference in refs {
            match run_check_direct(&site, contract, digest, reference, cancel.clone()).await {
                Ok(result) => results.push(result),
                Err(error) => self.note(format!(
                    "executability probe of '{}' could not run ({error}); it was not held back",
                    reference.acceptance_id
                )),
            }
        }
        results
    }

    /// The scratch observation the acceptance stage's guardian performs, run
    /// in-process over the unpublished candidate contract, at the target
    /// repository's HEAD, under the same repository lease.
    async fn observe(
        &self,
        binding: &NativeBinding,
        contract: &AcceptanceContract,
        digest: &str,
        refs: &[FrozenCommandRef],
        cancel: Arc<AtomicBool>,
    ) -> anyhow::Result<Vec<CheckResult>> {
        let identity = binding.policy.repository.canonicalize()?;
        let _lease = crate::command::acceptance_scratch_guardian::acquire_lease(
            &std::env::temp_dir().join("archon-native-observer-locks"),
            &identity.to_string_lossy(),
        )?;
        let head = git_head(&binding.policy.repository)
            .ok_or_else(|| anyhow::anyhow!("cannot read the repository HEAD"))?;
        let evidence = binding
            .policy
            .scratch_parent
            .join(format!("acceptance-probe-{}", uuid::Uuid::new_v4()));
        let observed = observe_commands_cancellable(
            &binding.policy,
            &head,
            contract,
            digest,
            refs,
            &evidence,
            cancel,
        )
        .await?;
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

fn stderr_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut start = text.len().saturating_sub(FINDING_TAIL_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].trim_end().to_string()
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
        crash_findings(contract, &results)
    }

    fn take_diagnostics(&self) -> Vec<String> {
        std::mem::take(&mut *self.diagnostics.lock().expect("diagnostics lock"))
    }
}

// The probe executes checks through the POSIX process-group runner.
#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_tests.rs"]
pub(crate) mod tests;
