//! Observation transaction: resolve, snapshot, execute, clean up, audit.
use super::hooks::{CHECK_DEFERRED, ObserveHooks};
#[path = "acceptance_scratch_isolate.rs"]
mod isolate;
use super::process::{CHECK_TIMED_OUT, CheckResult, run_at};
use super::*;
use crate::acceptance_world::{FrozenCommandRef, resolve_command};
use crate::task_set_contract::{AcceptanceContract, content_digest};
use isolate::normalized;
use std::sync::{Arc, atomic::AtomicBool};
#[path = "acceptance_scratch_observe_rebuild.rs"]
mod rebuild;
#[path = "acceptance_scratch_observe_result.rs"]
mod result_helpers;
use rebuild::Rebuild;
use result_helpers::{faulted, operational, redacted};

#[derive(Debug, Serialize, Deserialize)]
pub struct ObservationResult {
    pub checks: Vec<CheckResult>,
    pub host_environment: BTreeMap<String, bool>,
    pub check_evidence: Vec<CheckEvidence>,
    pub operational_errors: Vec<String>,
    pub command_refs: Vec<FrozenCommandRef>,
    pub command_cwds: Vec<crate::task_set_contract::TrustedCwd>,
    pub policy: ScratchPolicy,
    pub copied_project_manifest: BTreeMap<String, String>,
    pub cleanup_error: Option<String>,
    pub live_roots_unchanged: bool,
    pub teardown_verified: bool,
    pub source_commit: String,
    pub policy_digest: String,
    pub source_manifest_digest: String,
    pub before: BTreeMap<String, BTreeMap<String, String>>,
    pub after: BTreeMap<String, BTreeMap<String, String>>,
}
impl ObservationResult {
    pub fn passed(&self) -> bool {
        self.operational_errors.is_empty()
            && self.live_roots_unchanged
            && self.teardown_verified
            && !self.checks.is_empty()
            && self
                .checks
                .iter()
                .all(|c| c.exit_code == Some(0) && c.operational_error.is_none())
    }
}
/// The caller supplies an integrity-validated pinned contract and chain digest.
/// Authorization is repeated here before creating scratch or spawning children.
pub async fn observe_commands(
    policy: &ScratchPolicy,
    commit: &str,
    contract: &AcceptanceContract,
    chain_digest: &str,
    refs: &[FrozenCommandRef],
    evidence: &Path,
    resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
) -> WorkflowResult<ObservationResult> {
    observe_commands_cancellable(
        policy,
        commit,
        contract,
        chain_digest,
        refs,
        evidence,
        resource_limits,
        Arc::new(AtomicBool::new(false)),
    )
    .await
}
pub async fn observe_commands_cancellable(
    policy: &ScratchPolicy,
    commit: &str,
    contract: &AcceptanceContract,
    chain_digest: &str,
    refs: &[FrozenCommandRef],
    evidence: &Path,
    resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
    cancel: Arc<AtomicBool>,
) -> WorkflowResult<ObservationResult> {
    let hooks = ObserveHooks::for_resource_limits(resource_limits);
    observe_commands_hooked(
        policy,
        commit,
        contract,
        chain_digest,
        refs,
        evidence,
        cancel,
        &hooks,
    )
    .await
}
/// [`observe_commands_cancellable`] under the caller's [`ObserveHooks`].
#[allow(clippy::too_many_arguments)]
pub async fn observe_commands_hooked(
    policy: &ScratchPolicy,
    commit: &str,
    contract: &AcceptanceContract,
    chain_digest: &str,
    refs: &[FrozenCommandRef],
    evidence: &Path,
    cancel: Arc<AtomicBool>,
    hooks: &ObserveHooks,
) -> WorkflowResult<ObservationResult> {
    policy.validate()?;
    // Authorization is per check. Collecting these into one Result would let a
    // single command that cannot be authorized abort the batch here, before any
    // check runs and before the evidence directory is written at the end of
    // this function: every other selected check would then report a
    // guardian-level failure it did not cause, pointing at evidence that does
    // not exist. An unauthorized command is carried to its own check instead.
    let commands: Vec<WorkflowResult<_>> = refs
        .iter()
        .map(|r| resolve_command(contract, chain_digest, r))
        .collect();
    if commands.is_empty() {
        return Err(invalid(
            "no frozen commands selected for native observation",
        ));
    }
    for root in [&policy.repository, &policy.project, &policy.task_root] {
        if evidence.starts_with(root) {
            return Err(invalid("provisional evidence must be outside live roots"));
        }
    }
    let mut result = ObservationResult {
        checks: vec![],
        host_environment: policy
            .environment_allowlist
            .iter()
            .map(|key| (key.clone(), std::env::var_os(key).is_some()))
            .collect(),
        check_evidence: vec![],
        operational_errors: vec![],
        command_refs: refs.to_vec(),
        // Only the authorized commands have a trusted working directory; an
        // unauthorized one never reaches a site.
        command_cwds: commands
            .iter()
            .filter_map(|c| c.as_ref().ok().map(|c| c.cwd()))
            .collect(),
        policy: policy.clone(),
        copied_project_manifest: BTreeMap::new(),
        cleanup_error: None,
        live_roots_unchanged: false,
        teardown_verified: false,
        source_commit: commit.into(),
        policy_digest: content_digest(&serde_json::to_vec(policy)?),
        source_manifest_digest: String::new(),
        before: BTreeMap::new(),
        after: BTreeMap::new(),
    };
    let mut roots = None;
    let phase = || control::Control::new(policy.timeout_secs, cancel.clone());
    let execution = async {
        result.before = phase().run(|| inputs::live(policy, commit))?;
        // With no authorized command there is nothing to execute or provision.
        // Still record each denial and publish the ordinary observation evidence.
        if commands.iter().all(Result::is_err) {
            for (reference, command) in refs.iter().zip(commands) {
                if let Err(error) = command {
                    result
                        .checks
                        .push(operational(&reference.acceptance_id, error.to_string()));
                }
            }
            return Ok(());
        }
        // Resolved per check before any site exists: a sibling path no site
        // copies is that one check's finding, and the site is built for the
        // others (each check analysed as its own target).
        let refusals: Vec<Option<String>> = (commands.iter())
            .map(|command| {
                let command = command.as_ref().ok()?;
                siblings::refused_sibling(
                    &policy.project,
                    &policy.repository,
                    &String::from_utf8_lossy(command.bytes()),
                    command.cwd(),
                )
            })
            .collect();
        // Each check's own result before any site exists, and its index
        // among the site's checks when it has none.
        let pre_site = rebuild::pre_site(&commands, &refusals);
        let positions = rebuild::positions(&pre_site);
        let checks = (commands.iter().zip(&refusals))
            .filter(|(_, refused)| refused.is_none())
            .filter_map(|(command, _)| command.as_ref().ok())
            .map(|command| {
                (
                    String::from_utf8_lossy(command.bytes()).into_owned(),
                    command.cwd(),
                )
            })
            .collect::<Vec<_>>();
        roots =
            Some(phase().run(|| ScratchRoots::prepare_inner_with_checks(policy, commit, &checks))?);
        let prepared = roots.as_ref().expect("prepared");
        result.copied_project_manifest = phase().run(|| inventory(prepared.project()))?;
        let mut baseline = phase().run(|| identity::capture(prepared, policy))?;
        result.source_manifest_digest = content_digest(&serde_json::to_vec(
            &phase().run(|| prepared.source_inventory())?,
        )?);
        // Hashed in full once per site, then verified per check from metadata.
        let mut cargo_home = seal::TreeSeal::default();
        // Each check is its own unit (as a test runner gives each test its
        // own process): a fault one check leaves is its own result, and the next
        // check runs on a site restored to the baseline. Only a site that
        // cannot be restored stops the batch, as the host's.
        let mut dirty = false;
        let first = isolate::FirstSite::of(prepared, &baseline, &result.copied_project_manifest);
        let restore = |roots: &mut Option<ScratchRoots>| {
            let rebuilt = phase().run(|| isolate::restore(roots, policy, commit, &checks, &first));
            rebuild::classify(rebuilt, &cancel)
        };
        for (index, ((reference, command), refused)) in
            refs.iter().zip(commands).zip(&refusals).enumerate()
        {
            let Some((timeout_secs, cut)) = hooks.bound(policy.timeout_secs) else {
                for reference in &refs[index..] {
                    let deferred = operational(&reference.acceptance_id, CHECK_DEFERRED.into());
                    result.checks.push(deferred);
                }
                break;
            };
            let command = match command {
                Ok(command) => command,
                Err(error) => {
                    result
                        .checks
                        .push(operational(&reference.acceptance_id, error.to_string()));
                    continue;
                }
            };
            if let Some(refused) = refused {
                let cause = Some(faults::FaultCause::SiblingRootRefused);
                let own = faulted(&reference.acceptance_id, refused.clone(), cause);
                result.checks.push(own);
                continue;
            }
            // A check's fault, or what a check left that no fault named, is
            // followed by a site rebuilt once; a rebuilt site that still does
            // not verify is the host's, for the checks still to run.
            let mut restored = false;
            let before_identity = loop {
                let current = roots.as_ref().expect("prepared");
                let verified = (!dirty || restored).then(|| {
                    (phase().run(|| current.reset_project()))
                        .and_then(|()| phase().run(|| identity::capture(current, policy)))
                });
                let rebuilt = match verified {
                    Some(Ok(identity)) if identity == baseline => break Ok(identity),
                    _ if !restored => restore(&mut roots)?,
                    verified => Rebuild::HostUnproven(rebuild::unverified(verified, &cancel)?),
                };
                match rebuilt {
                    Rebuild::Verified(identity) => {
                        (baseline, restored) = (identity, true);
                        cargo_home = seal::TreeSeal::default();
                    }
                    Rebuild::HostUnproven(error) => break Err(error),
                }
            };
            let before_identity = match before_identity {
                Ok(identity) => identity,
                Err(error) => {
                    let later = rebuild::unproven(&refs[index..], &pre_site[index..], &error);
                    result.checks.extend(later);
                    break;
                }
            };
            let roots = roots.as_ref().expect("prepared");
            // A sibling this site could not copy for this check (`siblings`).
            if let Some((failed, cause)) = positions[index].and_then(|at| roots.sibling_failure(at))
            {
                let own = faulted(&reference.acceptance_id, failed.clone(), cause.clone());
                result.checks.push(own);
                dirty = false;
                continue;
            }
            let cargo_home_path = roots.root().join("cargo-home");
            let project_before = phase().run(|| inventory(roots.project()))?;
            let cache_before = phase().run(|| cargo_home.digest(&cargo_home_path))?;
            let target = roots.target();
            let site = isolate::check_site(
                roots,
                policy,
                &target,
                timeout_secs,
                hooks.resource_limits.clone(),
                hooks.mutation_plans.get(&reference.acceptance_id),
            );
            let attempt =
                execute_check_at(&site, contract, reference, &command, cancel.clone()).await;
            let mut check =
                attempt.unwrap_or_else(|e| operational(&reference.acceptance_id, e.to_string()));
            // Cut short by the caller's budget, not by its own length.
            let deferred = cut && check.operational_error.as_deref() == Some(CHECK_TIMED_OUT);
            if deferred {
                check.operational_error = Some(CHECK_DEFERRED.into());
            }
            // The check's own site after its own run: what it changed or
            // removed there is its own fault, recorded with its typed cause.
            let after_identity = phase()
                .run(|| Ok(identity::capture_site(roots, policy)))
                .unwrap_or_else(|e| Err(identity::CaptureFailure::Other(e)));
            let mut integrity_failed = isolate::judge_site(&mut check, &baseline, &after_identity);
            let project_after = phase().run(|| inventory(roots.project()));
            let cache_after = phase().run(|| cargo_home.digest(&cargo_home_path));
            let changed_paths = match project_after {
                Ok(after) => identity::changed(&project_before, &after),
                Err(e) => {
                    integrity_failed = true;
                    isolate::read_failed(&mut check, roots, "project", e);
                    vec![]
                }
            };
            let cache_after = cache_after.map_err(|e| {
                integrity_failed = true;
                isolate::read_failed(&mut check, roots, "build tool home", e);
            });
            result.check_evidence.push(CheckEvidence {
                acceptance_id: reference.acceptance_id.clone(),
                sibling_revisions: roots.sibling_revisions().to_vec(),
                before_identity,
                after_identity: after_identity.ok(),
                changed_project_paths: changed_paths,
                cargo_cache_before: cache_before,
                cargo_cache_after: cache_after.ok(),
                input_reset: true,
            });
            if let Some(on_check) = &hooks.on_check {
                on_check(&redacted(roots, &check));
            }
            // A check that changed its site or ended without a verdict (a
            // process it started may still use the site) is followed by a
            // restored site; it stops nothing.
            dirty = integrity_failed || (check.operational_error.is_some() && !deferred);
            let stop = deferred || cancel.load(std::sync::atomic::Ordering::SeqCst);
            result.checks.push(check);
            if deferred {
                for reference in &refs[index + 1..] {
                    let later = operational(&reference.acceptance_id, CHECK_DEFERRED.into());
                    result.checks.push(later);
                }
            }
            if stop {
                break;
            }
        }
        Ok::<_, WorkflowError>(())
    }
    .await;
    if let Err(e) = execution {
        result.operational_errors.push(e.to_string());
    }
    let cleanup = match roots.as_mut() {
        Some(roots) => roots.cleanup(),
        None => Ok(()), // prepare performs its own cleanup before returning an error.
    };
    result.teardown_verified = cleanup.is_ok();
    result.cleanup_error = cleanup.err().map(|e| e.to_string());
    if let Some(e) = &result.cleanup_error {
        result.operational_errors.push(e.clone());
    }
    let audit = control::Control::new(
        policy.timeout_secs,
        std::sync::Arc::new(AtomicBool::new(false)),
    );
    match audit.run(|| inputs::live(policy, commit)) {
        Ok(after) => {
            result.after = after;
            result.live_roots_unchanged = !result.before.is_empty()
                && normalized(&result.before) == normalized(&result.after);
        }
        Err(e) => result
            .operational_errors
            .push(format!("after audit failed: {e}")),
    }
    if !result.live_roots_unchanged && result.operational_errors.is_empty() {
        result
            .operational_errors
            .push("audited live inputs changed during observation".into());
    }
    for reference in refs {
        if !result
            .checks
            .iter()
            .any(|c| c.acceptance_id == reference.acceptance_id)
        {
            result.checks.push(operational(
                &reference.acceptance_id,
                result
                    .operational_errors
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "not executed after observation failure".into()),
            ));
        }
    }
    if let Some(roots) = &roots {
        for error in &mut result.operational_errors {
            *error = String::from_utf8_lossy(&roots.redact(error.as_bytes())).into_owned();
        }
        if let Some(error) = &mut result.cleanup_error {
            *error = String::from_utf8_lossy(&roots.redact(error.as_bytes())).into_owned();
        }
        for check in &mut result.checks {
            *check = redacted(roots, check);
        }
    }
    std::fs::create_dir_all(evidence).map_err(|e| WorkflowError::io(evidence, e))?;
    let path = evidence.join("observation.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&result)?)
        .map_err(|e| WorkflowError::io(&path, e))?;
    Ok(result)
}
/// One authorized check at a site: a nested verifier's declarative
/// prerequisites first (evaluated in-process, or as the generated predicate
/// command when they defer), then the pinned command itself.
pub async fn execute_check_at(
    site: &super::process::CommandSite<'_>,
    contract: &AcceptanceContract,
    reference: &FrozenCommandRef,
    command: &crate::acceptance_world::AuthorizedCommand,
    cancel: Arc<AtomicBool>,
) -> WorkflowResult<CheckResult> {
    if reference.kind == crate::acceptance_world::AcceptanceCommandKind::NestedVerifier {
        let entry = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .find(|e| e.id == reference.acceptance_id)
            .expect("authorized entry");
        if let crate::task_set_contract::AcceptanceCheck::Floor { contract: floor } = &entry.check {
            let mut prerequisites = floor.clone();
            prerequisites.typed_verifier_command = None;
            let facts = control::Control::new(site.timeout_secs, cancel.clone()).run(|| {
                crate::collect_declarative_floor_facts(
                    &crate::v2::deliverable_contract::ContractRoots::project_only(
                        site.project.to_string_lossy(),
                    ),
                    &prerequisites,
                )
            })?;
            match crate::evaluate_declarative_floor(&prerequisites, &facts) {
                crate::DeclarativeFloorEvaluation::Passed => {}
                crate::DeclarativeFloorEvaluation::Failed { findings } => {
                    return Ok(CheckResult {
                        fault: None,
                        classification: None,
                        acceptance_id: reference.acceptance_id.clone(),
                        exit_code: Some(1),
                        quota_walk_count: 0,
                        stdout: vec![],
                        stderr: findings.join("; ").into_bytes(),
                        operational_error: None,
                        environment_note: None,
                    });
                }
                crate::DeclarativeFloorEvaluation::Deferred { .. } => {
                    let generated =
                        crate::acceptance_world::AuthorizedCommand::floor_prerequisites(
                            site.project,
                            floor,
                        )?;
                    let checked =
                        run_at(site, &reference.acceptance_id, &generated, cancel.clone()).await?;
                    if checked.exit_code != Some(0) || checked.operational_error.is_some() {
                        return Ok(checked);
                    }
                }
            }
        }
    }
    run_at(site, &reference.acceptance_id, command, cancel).await
}
