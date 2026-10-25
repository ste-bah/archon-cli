//! Observation transaction: resolve, snapshot, execute, clean up, audit.
use super::hooks::{CHECK_DEFERRED, ObserveControl};
#[path = "acceptance_scratch_isolate.rs"]
mod isolate;
use super::process::{CHECK_TIMED_OUT, CheckResult};
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
pub use result_helpers::{AttributedTeardown, ObservationResult};
use result_helpers::{faulted, operational, redacted};
#[path = "acceptance_scratch_observe_execute.rs"]
mod execute;
pub use execute::execute_check_at;

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
        ObserveControl::for_resource_limits(Arc::new(AtomicBool::new(false)), resource_limits),
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
    control: ObserveControl,
) -> WorkflowResult<ObservationResult> {
    observe_commands_hooked(
        policy,
        commit,
        contract,
        chain_digest,
        refs,
        evidence,
        control,
    )
    .await
}
/// [`observe_commands_cancellable`] under the caller's observation control.
pub async fn observe_commands_hooked(
    policy: &ScratchPolicy,
    commit: &str,
    contract: &AcceptanceContract,
    chain_digest: &str,
    refs: &[FrozenCommandRef],
    evidence: &Path,
    control: ObserveControl,
) -> WorkflowResult<ObservationResult> {
    let ObserveControl { cancel, hooks } = control;
    let hooks = &hooks;
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
        attributed_teardown: None,
        live_roots_unchanged: false,
        teardown_verified: false,
        source_commit: commit.into(),
        policy_digest: content_digest(&serde_json::to_vec(policy)?),
        source_manifest_digest: String::new(),
        before: BTreeMap::new(),
        after: BTreeMap::new(),
    };
    let mut roots = None;
    let mut redaction_environment = BTreeMap::new();
    let mut dirty = false;
    let mut fresh = true;
    let mut dirty_owner: Option<usize> = None;
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
        redaction_environment = prepared.host_environment.clone();
        result.copied_project_manifest = phase().run(|| inventory(prepared.project()))?;
        let mut baseline = phase().run(|| identity::capture(prepared, policy))?;
        result.source_manifest_digest = content_digest(&serde_json::to_vec(
            &phase().run(|| prepared.source_inventory())?,
        )?);
        // The build tool home is shared by the checks and audited per check.
        let mut cargo_home = seal::TreeSeal::default();
        // Each check is its own unit (as a test runner gives each test its
        // own process): a fault one check leaves is its own result, and the next
        // check runs on a site restored to the baseline. A failed teardown
        // belongs to that check only when it started on a fresh verified site;
        // remaining checks then stay host-unproven.
        let first = isolate::FirstSite::of(prepared, &baseline, &result.copied_project_manifest);
        let restore = |roots: &mut Option<ScratchRoots>| {
            let rebuilt = phase().run(|| isolate::restore(roots, policy, commit, &checks, &first));
            match rebuilt {
                Err(teardown_error @ WorkflowError::ScratchSiteRemoval(_)) => {
                    Ok(Rebuild::CheckTeardown(teardown_error))
                }
                Ok(identity) => Ok(Rebuild::Verified(identity)),
                Err(error) => rebuild::classify(Err(error), &cancel),
            }
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
                    Some(Ok(identity)) if identity == baseline => {
                        dirty = false;
                        dirty_owner = None;
                        break Ok(identity);
                    }
                    _ if !restored => restore(&mut roots)?,
                    verified => Rebuild::HostUnproven(rebuild::unverified(verified, &cancel)?),
                };
                match rebuilt {
                    Rebuild::Verified(identity) => {
                        (baseline, restored) = (identity, true);
                        fresh = true;
                        dirty = false;
                        cargo_home = seal::TreeSeal::default();
                        dirty_owner = None;
                    }
                    Rebuild::HostUnproven(error) => break Err(error),
                    Rebuild::CheckTeardown(error) => {
                        phase().check()?;
                        if let Some(owner) = dirty_owner {
                            let site = roots.as_ref().expect("site teardown retains its roots");
                            let safe_error =
                                String::from_utf8_lossy(&site.redact(error.to_string().as_bytes()))
                                    .into_owned();
                            rebuild::attribute_teardown_failure(
                                &mut result.checks,
                                &refs[owner].acceptance_id,
                                &WorkflowError::ScratchSiteRemoval(safe_error.clone()),
                            );
                            result.attributed_teardown = Some(result_helpers::AttributedTeardown {
                                acceptance_id: refs[owner].acceptance_id.clone(),
                                error: safe_error.clone(),
                            });
                            for check in &mut result.checks {
                                *check = redacted(site, check);
                            }
                            roots = None;
                            dirty_owner = None;
                            break Err(WorkflowError::ScratchSiteRemoval(safe_error));
                        }
                        break Err(WorkflowError::ScratchSiteRemoval(format!(
                            "scratch site could not be restored for the remaining checks: {error}"
                        )));
                    }
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
            let site_was_fresh = fresh;
            fresh = false;
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
                cargo_cache_before: cache_before.clone(),
                cargo_cache_after: cache_after.as_ref().ok().cloned(),
                input_reset: true,
            });
            if let Some(on_check) = &hooks.on_check {
                on_check(&redacted(roots, &check));
            }
            // Keep the base rebuild rule; freshness independently proves this
            // check alone used the site if its teardown fails.
            dirty = integrity_failed || (check.operational_error.is_some() && !deferred);
            dirty_owner = (site_was_fresh && dirty).then_some(index);
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
    let cleanup_check_removal = matches!(&cleanup, Err(WorkflowError::ScratchSiteRemoval(_)));
    result.cleanup_error = cleanup.as_ref().err().map(ToString::to_string);
    let mut cleanup_attributed = false;
    if cleanup_check_removal && let Some(owner) = dirty_owner {
        let error = result.cleanup_error.as_deref().unwrap_or_default();
        rebuild::attribute_teardown_failure(
            &mut result.checks,
            &refs[owner].acceptance_id,
            &WorkflowError::ScratchSiteRemoval(error.to_owned()),
        );
        cleanup_attributed = true;
        result.attributed_teardown = Some(result_helpers::AttributedTeardown {
            acceptance_id: refs[owner].acceptance_id.clone(),
            error: error.to_owned(),
        });
    }
    result.teardown_verified = cleanup.is_ok() || cleanup_attributed;
    if let Some(e) = &result.cleanup_error {
        if !cleanup_attributed {
            result.operational_errors.push(e.clone());
        }
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
    for error in &mut result.operational_errors {
        *error =
            String::from_utf8_lossy(&io::redact(&redaction_environment, error.as_bytes(), false))
                .into_owned();
    }
    if let Some(error) = &mut result.cleanup_error {
        *error =
            String::from_utf8_lossy(&io::redact(&redaction_environment, error.as_bytes(), false))
                .into_owned();
    }
    if let Some(teardown) = &mut result.attributed_teardown {
        teardown.error = String::from_utf8_lossy(&io::redact(
            &redaction_environment,
            teardown.error.as_bytes(),
            false,
        ))
        .into_owned();
    }
    for check in &mut result.checks {
        *check = result_helpers::redacted_environment(&redaction_environment, check);
    }
    std::fs::create_dir_all(evidence).map_err(|e| WorkflowError::io(evidence, e))?;
    let path = evidence.join("observation.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&result)?)
        .map_err(|e| WorkflowError::io(&path, e))?;
    Ok(result)
}
