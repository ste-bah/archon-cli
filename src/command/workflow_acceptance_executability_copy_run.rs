//! Shared check execution for disposable copies.
use super::*;
pub(crate) async fn run_refs_in_copy(
    probe: &HostProbe,
    copy: &HermeticCopy,
    repository: &Path,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
    hooks: &archon_workflow::acceptance_scratch::ObserveHooks,
    observed_layout: &std::sync::Mutex<Option<CopyLayout>>,
    cancel: Arc<AtomicBool>,
) -> Result<CopyRun, Unrun> {
    if let Some(guard) = &copy.base_guard {
        guard.before_layout(contract)?;
    }
    let mut layout = copy.layout();
    siblings::install_copy_layout(
        &mut layout,
        &copy.project,
        &probe.project,
        &copy.repository,
        repository,
        contract,
    )?;
    if let Some(guard) = &copy.base_guard {
        guard.tripwire(copy)?;
    }
    *observed_layout.lock().expect("observation layout lock") = Some(layout.clone());
    let target = copy
        .base_guard
        .as_ref()
        .map(|g| g.build_output.clone())
        .unwrap_or_else(|| warm_target(&probe.copy_parent, repository));
    let mut site = DirectSite {
        repository: copy.repository.clone(),
        project: copy.project.clone(),
        host: probe.host.clone(),
        policy: None,
        resource_limits: probe.resource_limits.clone(),
        target: Some(target),
        fresh_home: true,
        timeout_secs: probe.check_bound_secs(),
        output_bytes: DIRECT_DEFAULT_OUTPUT_BYTES,
    };
    let budget = if let Site::Scratch(binding) = &probe.site {
        let (environment, _) = sites::site_environment(probe);
        site.host = environment;
        site.policy = Some(
            archon_workflow::acceptance_check_environment::CheckPolicy::configured(&binding.policy),
        );
        site.timeout_secs = site.timeout_secs.min(binding.policy.timeout_secs);
        site.output_bytes = binding.policy.output_bytes;
        Some((copy.root.as_path(), binding.policy.scratch_bytes))
    } else {
        None
    };
    let site_time_cap = site.timeout_secs;
    let mut results = Vec::new();
    let mut failure = None;
    for (index, reference) in refs.iter().enumerate() {
        let allowance = hooks.allowance.as_ref().map(|allowance| allowance());
        let cut = match allowance {
            Some(CheckAllowance::Defer) => {
                results.extend(refs[index..].iter().map(|r| deferred(&r.acceptance_id)));
                break;
            }
            Some(CheckAllowance::Run { timeout_secs, cut }) => {
                site.timeout_secs = timeout_secs
                    .clamp(1, probe.check_bound_secs())
                    .min(site_time_cap);
                cut
            }
            None => false,
        };
        let execution = async {
            #[cfg(test)]
            crate::command::workflow_host_fault_test_hooks::fire(
                crate::command::workflow_host_fault_test_hooks::Point::Direct,
                repository,
            )
            .map_err(|source| archon_workflow::WorkflowError::Io {
                path: repository.to_owned(),
                source,
            })?;
            archon_workflow::acceptance_scratch::run_check_direct_bounded(
                &site,
                contract,
                digest,
                reference,
                hooks.mutation_plans.get(&reference.acceptance_id),
                budget,
                cancel.clone(),
            )
            .await
        }
        .await;
        match execution {
            Ok(mut result) => {
                let stopped = cut && result.operational_error.as_deref() == Some(CHECK_TIMED_OUT);
                if stopped {
                    result.operational_error = Some(CHECK_DEFERRED.to_string());
                }
                if let Some(on_check) = &hooks.on_check {
                    on_check(&result);
                }
                results.push(result);
                if stopped {
                    results.extend(refs[index + 1..].iter().map(|r| deferred(&r.acceptance_id)));
                    break;
                }
            }
            Err(error) => {
                let context = format!(
                    "running check {} in the disposable site {}",
                    reference.acceptance_id,
                    site.project.display()
                );
                failure = Some(Unrun::error(error.into(), context));
                break;
            }
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(CopyRun { results, layout }),
    }
}
