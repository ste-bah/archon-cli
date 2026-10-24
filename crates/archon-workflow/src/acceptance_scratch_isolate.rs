//! Per-check isolation of one observation batch.
//!
//! Each check is its own unit, as a test runner gives each test its own
//! process: a check that changes its site, removes a part of it, or leaves a
//! process that may still hold it is given that fault as its own result,
//! and the site is restored before the next check runs. Only a site that
//! cannot be restored for anyone stops the batch, and that is the host's.
use super::faults::{FaultCause, identity_changed};
use super::identity::{BuildIdentity, CaptureFailure};
use super::*;

/// Record on `check` the fault its own run left at its site, judged from
/// `after` (the site's identity after the run) against `baseline`, with its
/// typed cause where it is the run's; whether the site no longer verifies.
pub(super) fn judge_site(
    check: &mut CheckResult,
    baseline: &BuildIdentity,
    after: &Result<BuildIdentity, CaptureFailure>,
) -> bool {
    let (text, cause) = match after {
        Ok(current) if current == baseline => return false,
        Ok(current) => {
            let parts = baseline.changed_parts(current);
            let named = parts.iter().map(|part| part.to_string()).collect();
            let cause = FaultCause::IdentityChanged { parts: named };
            (identity_changed(&parts), Some(cause))
        }
        Err(CaptureFailure::Removed { part, path }) => removed(part, path),
        Err(CaptureFailure::TargetMapping) => (
            uncaptured(&CaptureFailure::TargetMapping.into_error()),
            Some(FaultCause::TargetMappingChanged),
        ),
        Err(CaptureFailure::Other(e)) => (uncaptured(e), None),
    };
    check.operational_error = Some(text);
    check.fault = cause;
    true
}

/// Record on `check` that its site could not be read after its run: a
/// part its own run removed is its fault, a fault already recorded stays
/// (what the run did explains the failed read), anything else the host's.
pub(super) fn read_failed(
    check: &mut CheckResult,
    roots: &ScratchRoots,
    part: &'static str,
    error: WorkflowError,
) {
    if check.fault.is_some() {
        return;
    }
    let (text, cause) = match CaptureFailure::of(roots, part, error) {
        CaptureFailure::Removed { part, path } => removed(part, &path),
        other => (other.into_error().to_string(), None),
    };
    check.operational_error = Some(text);
    check.fault = cause;
}

fn removed(part: &str, path: &Path) -> (String, Option<FaultCause>) {
    let text = format!(
        "{} after the check removed {}",
        identity_changed(&[part]),
        path.display()
    );
    let part = part.to_string();
    (text, Some(FaultCause::Removed { part }))
}

fn uncaptured(error: &WorkflowError) -> String {
    format!(
        "native scratch source or build identity could not be captured after the check ({error}); warm target reuse refused"
    )
}

/// What a batch's first site was, on every part that does not depend on
/// where the site is laid out: a restored site must match it.
pub(super) struct FirstSite {
    identity: BuildIdentity,
    manifest: BTreeMap<String, String>,
    root: PathBuf,
    target: PathBuf,
}
impl FirstSite {
    pub(super) fn of(
        roots: &ScratchRoots,
        identity: &BuildIdentity,
        manifest: &BTreeMap<String, String>,
    ) -> Self {
        Self {
            identity: identity.clone(),
            manifest: site_neutral(roots, manifest),
            root: roots.root().to_path_buf(),
            target: roots.target(),
        }
    }
}

/// `manifest`, of the project copy of `roots`, on what does not depend on
/// where or beside what the site is laid out: its paths are named, not
/// spelled, and a `.git` link to the site's own worktree (the combined
/// view's) is named by what it resolves to, never by the administrative
/// directory name the live repository picked for it.
fn site_neutral(
    roots: &ScratchRoots,
    manifest: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let own_link = roots.links_own_worktree(&roots.project().join(".git"));
    (manifest.iter())
        .map(|(key, value)| match key.as_str() {
            ".git" if own_link => (key.clone(), "<the site's own worktree link>".into()),
            _ => (
                key.clone(),
                identity::neutral(value, roots.root(), &roots.target()),
            ),
        })
        .collect()
}

/// Tear `roots` down and prepare a fresh site for `checks` under the same
/// build cache lease (held throughout), returning its baseline identity. The warm build output is kept only when no process
/// the last check started can still reach it (`forget_unless_idle`); else
/// the fresh site and its build output are at paths no such process knows.
/// The fresh site must match `first` on every part that does not depend on
/// its path (source, copied project inputs, tools, build configuration,
/// environment): a difference is the host's change under the batch. A site
/// that cannot be torn down stays in `roots`, so the observation's own
/// cleanup reports it. Every failure here is the host's and stops the batch.
pub(super) fn restore(
    roots: &mut Option<ScratchRoots>,
    policy: &ScratchPolicy,
    commit: &str,
    checks: &[(String, crate::task_set_contract::TrustedCwd)],
    first: &FirstSite,
) -> WorkflowResult<BuildIdentity> {
    let mut lease = None;
    if let Some(current) = roots.as_mut() {
        lease = (current.forget_unless_idle())
            .and_then(|()| current.tear_down_keeping_lease())
            .map_err(|e| {
                invalid(format!(
                    "the scratch site could not be restored for the remaining checks: {e}"
                ))
            })?;
    }
    *roots = None;
    let fresh = ScratchRoots::prepare_leased(policy, commit, checks, lease)?;
    let identity = identity::capture(&fresh, policy)?;
    let at = (fresh.root().to_path_buf(), fresh.target());
    let mut changed =
        first
            .identity
            .host_parts_changed((&first.root, &first.target), &identity, (&at.0, &at.1));
    if first.manifest != site_neutral(&fresh, &inventory(fresh.project())?) {
        changed.push("copied project inputs");
    }
    *roots = Some(fresh);
    if !changed.is_empty() {
        return Err(invalid(format!(
            "the restored scratch site differs from the batch's first site ({}): the host changed under the observation",
            changed.join(", ")
        )));
    }
    Ok(identity)
}

/// `maps` without an empty `.git/worktrees` directory entry, which the
/// scratch worktree's registration leaves behind.
pub(super) fn normalized(
    maps: &BTreeMap<String, BTreeMap<String, String>>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut maps = maps.clone();
    for entries in maps.values_mut() {
        if entries
            .get(".git/worktrees")
            .is_some_and(|v| v == "directory")
            && !entries.keys().any(|p| p.starts_with(".git/worktrees/"))
        {
            entries.remove(".git/worktrees");
        }
    }
    maps
}
/// The scratch site one check runs at, for at most `timeout_secs`.
pub(super) fn check_site<'a>(
    roots: &'a ScratchRoots,
    policy: &ScratchPolicy,
    target: &'a Path,
    timeout_secs: u64,
    resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
    mutation_plan: Option<&'a super::process::MutationPlan>,
) -> super::process::CommandSite<'a> {
    super::process::CommandSite {
        project: roots.project(),
        repository: roots.repository(),
        environment: roots.command_environment(policy),
        resource_limits,
        audit_root: Some(roots.root()),
        audit_target: Some(target),
        scratch_bytes: policy.scratch_bytes,
        output_bytes: policy.output_bytes,
        timeout_secs,
        mutation_plan,
        mutation_root: Some(roots.mutation_root().clone()),
        redactor: Some(roots),
    }
}
