//! Per-check isolation of one observation batch.
//!
//! Each check is its own unit, as a test runner gives each test its own
//! process: a check that changes its site, removes a part of it, or leaves a
//! process that may still hold it is given that fault as its own result,
//! and the site is restored before the next check runs. Only a site that
//! cannot be restored for anyone stops the batch, and that is the host's.
use super::identity::{BuildIdentity, CaptureFailure};
use super::*;

/// The operational error of a check whose own site could not be captured
/// after its run: a part it removed is named as the identity part it
/// changed, which `faults::check_fault` gives to the check.
pub(super) fn capture_failed(failure: &CaptureFailure) -> String {
    match failure {
        CaptureFailure::Removed { part, path } => format!(
            "{} after the check removed {}",
            super::faults::identity_changed(&[part]),
            path.display()
        ),
        CaptureFailure::Other(e) => format!(
            "native scratch source or build identity could not be captured after the check ({e}); warm target reuse refused"
        ),
    }
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
            manifest: manifest.clone(),
            root: roots.root().to_path_buf(),
            target: roots.target(),
        }
    }
}

/// Tear `roots` down and prepare a fresh site for `checks`, returning its
/// baseline identity. The warm build output is kept only when no process
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
    if let Some(current) = roots.as_mut() {
        current
            .forget_unless_idle()
            .and_then(|()| current.cleanup())
            .map_err(|e| {
                invalid(format!(
                    "the scratch site could not be restored for the remaining checks: {e}"
                ))
            })?;
    }
    *roots = None;
    let fresh = ScratchRoots::prepare_inner_with_checks(policy, commit, checks)?;
    let identity = identity::capture(&fresh, policy)?;
    let at = (fresh.root().to_path_buf(), fresh.target());
    let mut changed =
        first
            .identity
            .host_parts_changed((&first.root, &first.target), &identity, (&at.0, &at.1));
    let manifest = |map: &BTreeMap<String, String>, (root, target): (&Path, &Path)| {
        (map.iter())
            .map(|(key, value)| (key.clone(), identity::neutral(value, root, target)))
            .collect::<BTreeMap<_, _>>()
    };
    if manifest(&first.manifest, (&first.root, &first.target))
        != manifest(&inventory(fresh.project())?, (&at.0, &at.1))
    {
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
