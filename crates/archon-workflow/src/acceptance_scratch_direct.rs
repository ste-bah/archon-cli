//! Direct acceptance execution in live roots, for a run with no
//! `[workflow.acceptance_execution]` policy.
//!
//! The scratch observer is opt-in: it needs a configured toolchain path,
//! scratch parent and limits, and it pays for hermeticity with a cold build
//! cache. A run whose project never opted in still has frozen checks that
//! must run before it may complete (Obs-32), so this site runs them where the
//! run's own agents ran theirs — in the target repository checkout, with the
//! environment the one check-environment rule gives every site
//! (`acceptance_check_environment`, Issue 345): built from the host
//! environment the caller hands over, never that environment itself, with
//! the host's HOME in the live checkout (no filesystem sandbox makes a fresh
//! one a boundary there) and a fresh one per check in a probe's copy --
//! through the SAME bounded process-group runner the
//! scratch observer uses. There is one runner; this is the second place it
//! is pointed at.
//!
//! Every result says which mode produced it: the record the caller writes
//! carries the site, and a reader can tell a hermetic observation from a
//! direct one.

use super::process::CommandSite;
use super::*;
use crate::acceptance_check_environment::{
    CheckPolicy, check_environment, profile_bindings, withheld, withheld_error,
};
use crate::acceptance_world::{FrozenCommandRef, resolve_command};
use crate::task_set_contract::AcceptanceContract;
use std::collections::BTreeSet;
use std::sync::{Arc, atomic::AtomicBool};

/// Bounds a direct site inherits when no policy configured them.
pub const DIRECT_DEFAULT_TIMEOUT_SECS: u64 = 1800;
pub const DIRECT_DEFAULT_OUTPUT_BYTES: usize = 256 * 1024;

/// Where a direct run executes and what it may spend.
#[derive(Clone)]
pub struct DirectSite {
    pub repository: PathBuf,
    pub project: PathBuf,
    /// The host environment a check's environment is built from (Issue 345):
    /// a check gets only what its site's policy allows of it.
    pub host: BTreeMap<String, String>,
    /// The configured policy, when a section is configured (a floor at a
    /// scratch stage); `None` is the default policy of a site with none.
    pub policy: Option<CheckPolicy>,
    /// The site's own build directory, when it has one (a probe's copy).
    pub target: Option<PathBuf>,
    /// A fresh HOME per check (a probe's copy), else the host's (the live
    /// checkout, where a check can read the host's home by path anyway).
    pub fresh_home: bool,
    pub timeout_secs: u64,
    pub output_bytes: usize,
}

/// The host's variable names only: its values may be secrets.
impl std::fmt::Debug for DirectSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectSite")
            .field("repository", &self.repository)
            .field("project", &self.project)
            .field("host", &self.host.keys().collect::<Vec<_>>())
            .field("policy", &self.policy.as_ref().map(|p| &p.forwarded))
            .field("target", &self.target)
            .field("fresh_home", &self.fresh_home)
            .field("timeout_secs", &self.timeout_secs)
            .field("output_bytes", &self.output_bytes)
            .finish()
    }
}

/// One check's fresh HOME, removed after it ran.
struct FreshHome(PathBuf);

impl FreshHome {
    fn new() -> WorkflowResult<Self> {
        let path = std::env::temp_dir().join(format!("archon-check-home-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|e| WorkflowError::io(&path, e))?;
        Ok(Self(path))
    }
}

impl Drop for FreshHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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

    /// A check's environment here, with `home` as its fresh HOME (else the
    /// host's), and the host variables it is not given.
    pub fn check_environment(
        &self,
        home: Option<&Path>,
    ) -> WorkflowResult<(BTreeMap<String, String>, BTreeSet<String>)> {
        let policy = (self.policy.clone()).unwrap_or_else(|| CheckPolicy::default_for(&self.host));
        let host_home = (self.host.get("HOME")).map(Path::new);
        let mut site: Vec<(&str, &Path)> = home
            .or(host_home)
            .map(|h| ("HOME", h))
            .into_iter()
            .collect();
        site.extend(
            self.target
                .as_deref()
                .map(|target| ("CARGO_TARGET_DIR", target)),
        );
        // Windows: a fresh home is the profile too; the live checkout keeps
        // the host's profile.
        let profile = home.map(profile_bindings).unwrap_or_default();
        site.extend(profile.iter().map(|(name, path)| (*name, path.as_path())));
        let environment = check_environment(&self.host, &policy, &site).map_err(invalid)?;
        let withheld = withheld(&self.host, &environment);
        Ok((environment, withheld))
    }

    /// One check's environment: its fresh HOME, if the site gives one (held
    /// until the check ends), the variables, and what is withheld.
    fn prepare(
        &self,
    ) -> WorkflowResult<(
        Option<FreshHome>,
        BTreeMap<String, String>,
        BTreeSet<String>,
    )> {
        let home = if self.fresh_home {
            Some(FreshHome::new()?)
        } else {
            None
        };
        let (environment, withheld) =
            self.check_environment(home.as_ref().map(|h| h.0.as_path()))?;
        Ok((home, environment, withheld))
    }

    fn command_site(&self, environment: BTreeMap<String, String>) -> CommandSite<'_> {
        CommandSite {
            project: &self.project,
            repository: &self.repository,
            environment,
            audit_root: None,
            audit_target: None,
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
    let (_home, environment, withheld) = site.prepare()?;
    let at = site.command_site(environment);
    let result =
        super::observe::execute_check_at(&at, contract, reference, &command, cancel).await?;
    Ok(unless_withheld(result, &withheld))
}

/// `result`, or, when it failed saying a withheld variable the allowlist
/// could forward is missing, the operational error naming it (no verdict).
fn unless_withheld(mut result: CheckResult, withheld: &BTreeSet<String>) -> CheckResult {
    if result.exit_code != Some(0) && result.operational_error.is_none() {
        result.operational_error = withheld_error(&[&result.stdout, &result.stderr], withheld);
    }
    result
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
            classification: None,
            acceptance_id: acceptance_id.into(),
            exit_code: Some(0),
            quota_walk_count: 0,
            stdout: b"declarative floor satisfied".to_vec(),
            stderr: vec![],
            operational_error: None,
        }),
        crate::DeclarativeFloorEvaluation::Failed { findings } => Ok(CheckResult {
            classification: None,
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
            let (_home, environment, withheld) = site.prepare()?;
            let at = site.command_site(environment);
            let result = super::process::run_at(&at, acceptance_id, &generated, cancel).await?;
            Ok(unless_withheld(result, &withheld))
        }
    }
}

#[cfg(all(test, unix))]
#[path = "acceptance_scratch_direct_env_tests.rs"]
mod env_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acceptance_world::AcceptanceCommandKind;
    use crate::task_set_contract::{
        AcceptanceCheck, AcceptanceCriterion, GapPolicy, JudgeDecision, JudgeVerdict, PrdIdentity,
        TrustedCwd, content_digest,
    };

    pub(super) fn criterion(id: &str, command: &str, cwd: TrustedCwd) -> AcceptanceCriterion {
        AcceptanceCriterion {
            id: id.into(),
            criterion: format!("criterion {id}"),
            check: AcceptanceCheck::Command {
                command: command.into(),
                cwd,
            },
            gap_permitted: false,
            covers: Vec::new(),
            judgment: JudgeVerdict {
                verdict: JudgeDecision::Accepted,
                counterexample: "missing".into(),
                reason: "declared".into(),
                host_call_id: "judge-1".into(),
                sampling: None,
            },
        }
    }

    pub(super) fn contract(criteria: Vec<AcceptanceCriterion>) -> (AcceptanceContract, String) {
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

    pub(super) fn reference(
        contract: &AcceptanceContract,
        digest: &str,
        id: &str,
    ) -> FrozenCommandRef {
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

    pub(super) fn site(repository: &Path, project: &Path) -> DirectSite {
        DirectSite {
            repository: repository.to_path_buf(),
            project: project.to_path_buf(),
            host: BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
            policy: None,
            target: None,
            fresh_home: false,
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

    /// Round 2 (P2): a configured limit too large for the clock is no
    /// deadline at all, never an overflow that panics the host.
    #[tokio::test]
    #[cfg(unix)]
    async fn a_limit_past_the_clock_is_no_deadline_and_never_panics() {
        let repo = tempfile::tempdir().unwrap();
        let (contract, digest) =
            contract(vec![criterion("REQ-1", "test -d .", TrustedCwd::RepoRoot)]);
        let mut site = site(repo.path(), repo.path());
        site.timeout_secs = i64::MAX as u64;
        let result = run_check_direct(
            &site,
            &contract,
            &digest,
            &reference(&contract, &digest, "REQ-1"),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, Some(0), "{result:?}");
    }
}
