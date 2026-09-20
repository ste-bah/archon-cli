//! Direct acceptance execution in live roots, for a run with no
//! `[workflow.acceptance_execution]` policy.
//!
//! The scratch observer is opt-in: it needs a configured toolchain path,
//! scratch parent and limits, and it pays for hermeticity with a cold build
//! cache. A run whose project never opted in still has frozen checks that
//! must run before it may complete (Obs-32), so this site runs them where the
//! run's own agents ran theirs — in the target repository checkout, with the
//! host environment the caller hands over — through the SAME bounded
//! process-group runner the scratch observer uses. There is one runner; this
//! is the second place it is pointed at.
//!
//! Every result says which mode produced it: the record the caller writes
//! carries the site, and a reader can tell a hermetic observation from a
//! direct one.

use super::process::CommandSite;
use super::*;
use crate::acceptance_world::{FrozenCommandRef, resolve_command};
use crate::task_set_contract::AcceptanceContract;
use std::sync::{Arc, atomic::AtomicBool};

/// Bounds a direct site inherits when no policy configured them.
pub const DIRECT_DEFAULT_TIMEOUT_SECS: u64 = 1800;
pub const DIRECT_DEFAULT_OUTPUT_BYTES: usize = 256 * 1024;

/// Where a direct run executes and what it may spend.
#[derive(Clone, Debug)]
pub struct DirectSite {
    pub repository: PathBuf,
    pub project: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub timeout_secs: u64,
    pub output_bytes: usize,
}

impl DirectSite {
    pub fn validate(&self) -> WorkflowResult<()> {
        for root in [&self.repository, &self.project] {
            if !root.is_absolute() || !root.is_dir() {
                return Err(invalid(format!(
                    "direct acceptance site root {} must be an existing absolute directory",
                    root.display()
                )));
            }
        }
        if self.timeout_secs == 0 || self.output_bytes == 0 {
            return Err(invalid("direct acceptance site limits must be positive"));
        }
        Ok(())
    }

    fn command_site(&self) -> CommandSite<'_> {
        CommandSite {
            project: &self.project,
            repository: &self.repository,
            environment: self.environment.clone(),
            audit_root: None,
            scratch_bytes: u64::MAX,
            output_bytes: self.output_bytes,
            timeout_secs: self.timeout_secs,
            redactor: None,
        }
    }
}

/// Run one pinned command-bearing check at the site. The reference is
/// resolved against the contract the same way the scratch observer resolves
/// it, so a command that is not in the pinned chain is refused here too.
pub async fn run_check_direct(
    site: &DirectSite,
    contract: &AcceptanceContract,
    chain_digest: &str,
    reference: &FrozenCommandRef,
    cancel: Arc<AtomicBool>,
) -> WorkflowResult<CheckResult> {
    site.validate()?;
    let command = resolve_command(contract, chain_digest, reference)?;
    super::observe::execute_check_at(&site.command_site(), contract, reference, &command, cancel)
        .await
}

/// Evaluate a declarative floor (a `Floor` check with no verifier command) at
/// the site: in-process when its predicates are evaluable, otherwise as the
/// host-generated predicate command — the same two paths the scratch observer
/// takes for a nested verifier's prerequisites. Always a verdict, never a
/// deferral: an acceptance check that cannot be evaluated is not a pass.
pub async fn evaluate_floor_direct(
    site: &DirectSite,
    acceptance_id: &str,
    floor: &crate::task_universe::WorkflowV2DeliverableContract,
    cancel: Arc<AtomicBool>,
) -> WorkflowResult<CheckResult> {
    site.validate()?;
    let roots = crate::v2::deliverable_contract::ContractRoots::project_only(
        site.project.to_string_lossy(),
    );
    let facts = crate::collect_declarative_floor_facts(&roots, floor)?;
    match crate::evaluate_declarative_floor(floor, &facts) {
        crate::DeclarativeFloorEvaluation::Passed => Ok(CheckResult {
            acceptance_id: acceptance_id.into(),
            exit_code: Some(0),
            quota_walk_count: 0,
            stdout: b"declarative floor satisfied".to_vec(),
            stderr: vec![],
            operational_error: None,
        }),
        crate::DeclarativeFloorEvaluation::Failed { findings } => Ok(CheckResult {
            acceptance_id: acceptance_id.into(),
            exit_code: Some(1),
            quota_walk_count: 0,
            stdout: vec![],
            stderr: findings.join("; ").into_bytes(),
            operational_error: None,
        }),
        crate::DeclarativeFloorEvaluation::Deferred { .. } => {
            let generated = crate::acceptance_world::AuthorizedCommand::floor_prerequisites(
                &site.project,
                floor,
            )?;
            super::process::run_at(&site.command_site(), acceptance_id, &generated, cancel).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acceptance_world::AcceptanceCommandKind;
    use crate::task_set_contract::{
        AcceptanceCheck, AcceptanceCriterion, GapPolicy, JudgeDecision, JudgeVerdict, PrdIdentity,
        TrustedCwd, content_digest,
    };

    fn criterion(id: &str, command: &str, cwd: TrustedCwd) -> AcceptanceCriterion {
        AcceptanceCriterion {
            id: id.into(),
            criterion: format!("criterion {id}"),
            check: AcceptanceCheck::Command {
                command: command.into(),
                cwd,
            },
            gap_permitted: false,
            judgment: JudgeVerdict {
                verdict: JudgeDecision::Accepted,
                counterexample: "missing".into(),
                reason: "declared".into(),
                host_call_id: "judge-1".into(),
                sampling: None,
            },
        }
    }

    fn contract(criteria: Vec<AcceptanceCriterion>) -> (AcceptanceContract, String) {
        let contract = AcceptanceContract {
            schema_version: 1,
            prd: PrdIdentity {
                path: "prd.md".into(),
                digest: "d".into(),
            },
            gap_policy: GapPolicy {
                permitted_acceptance_ids: Default::default(),
                forbidden_phrases: Vec::new(),
                required_fields: Vec::new(),
            },
            acceptance: criteria,
            supplementary: Vec::new(),
        };
        let digest = content_digest(&serde_json::to_vec(&contract).unwrap());
        (contract, digest)
    }

    fn reference(contract: &AcceptanceContract, digest: &str, id: &str) -> FrozenCommandRef {
        let entry = contract.acceptance.iter().find(|e| e.id == id).unwrap();
        let AcceptanceCheck::Command { command, .. } = &entry.check else {
            unreachable!()
        };
        FrozenCommandRef {
            acceptance_id: id.into(),
            kind: AcceptanceCommandKind::Command,
            chain_digest: digest.into(),
            command_digest: content_digest(command.as_bytes()),
        }
    }

    fn site(repository: &Path, project: &Path) -> DirectSite {
        DirectSite {
            repository: repository.to_path_buf(),
            project: project.to_path_buf(),
            environment: BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
            timeout_secs: 20,
            output_bytes: 4096,
        }
    }

    /// Commands run in the live roots the pinned cwd names, and pass/fail
    /// comes back as the exit code with output captured.
    #[tokio::test]
    #[cfg(unix)] // Requires Unix process-group teardown, not just leader termination.
    async fn a_direct_check_runs_in_the_pinned_root_and_reports_its_exit() {
        let repo = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("present"), "x").unwrap();
        let (contract, digest) = contract(vec![
            criterion("REQ-1", "test -f present && echo ok", TrustedCwd::RepoRoot),
            criterion("REQ-2", "test -f present", TrustedCwd::ProjectRoot),
        ]);
        let site = site(repo.path(), project.path());
        let cancel = Arc::new(AtomicBool::new(false));
        let passed = run_check_direct(
            &site,
            &contract,
            &digest,
            &reference(&contract, &digest, "REQ-1"),
            cancel.clone(),
        )
        .await
        .unwrap();
        assert_eq!(passed.exit_code, Some(0), "{passed:?}");
        assert_eq!(String::from_utf8_lossy(&passed.stdout).trim(), "ok");
        assert!(passed.operational_error.is_none());
        let failed = run_check_direct(
            &site,
            &contract,
            &digest,
            &reference(&contract, &digest, "REQ-2"),
            cancel,
        )
        .await
        .unwrap();
        assert_eq!(failed.exit_code, Some(1), "{failed:?}");
    }

    /// A command outside the pinned chain is refused before anything runs.
    #[tokio::test]
    async fn a_reference_that_is_not_in_the_pinned_chain_is_refused() {
        let repo = tempfile::tempdir().unwrap();
        let (contract, digest) =
            contract(vec![criterion("REQ-1", "test -d .", TrustedCwd::RepoRoot)]);
        let mut forged = reference(&contract, &digest, "REQ-1");
        forged.command_digest = content_digest(b"rm -rf /");
        let error = run_check_direct(
            &site(repo.path(), repo.path()),
            &contract,
            &digest,
            &forged,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect_err("digest mismatch");
        assert!(error.to_string().contains("digest"), "{error}");
    }

    #[tokio::test]
    #[cfg(unix)] // Requires Unix process-group teardown, not just leader termination.
    async fn a_timed_out_direct_check_is_an_operational_error_not_a_pass() {
        let repo = tempfile::tempdir().unwrap();
        let (contract, digest) =
            contract(vec![criterion("REQ-1", "sleep 30", TrustedCwd::RepoRoot)]);
        let mut site = site(repo.path(), repo.path());
        site.timeout_secs = 1;
        let result = run_check_direct(
            &site,
            &contract,
            &digest,
            &reference(&contract, &digest, "REQ-1"),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert!(result.operational_error.is_some(), "{result:?}");
        assert_ne!(result.exit_code, Some(0));
    }
}
