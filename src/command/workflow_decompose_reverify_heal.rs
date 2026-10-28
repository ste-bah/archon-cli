//! A completed run's acceptance observation retries before report publication.
use super::*;
use crate::command::workflow_host_retry;

pub(super) async fn checks(
    control: workflow_host_retry::Control<'_>,
    (project_root, prd_path, task_root): (&Path, &Path, &Path),
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<acceptance::ReverifyChecks> {
    workflow_host_retry::retry("reverify", control, || async {
        let checks =
            acceptance::reverify_checks(project_root, prd_path, task_root, config, env_vars)
                .await?;
        if checks.operational_failure {
            let cause = checks
                .checks
                .iter()
                .filter(|check| check.verdict == "operational_failure")
                .flat_map(|check| &check.findings)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ");
            if checks.transient_only
                && checks.checks.iter().all(|check| {
                    matches!(check.verdict.as_str(), "operational_failure" | "accepted")
                })
            {
                return Err(crate::command::workflow_host_fault::transient(cause));
            }
        }
        Ok(checks)
    })
    .await
}

pub(super) fn transient_observations(
    accepted_ids: &std::collections::BTreeSet<String>,
    evidence: &[crate::command::workflow_task_set::executability::CheckEvidence],
    causes: &std::collections::BTreeMap<
        String,
        (
            String,
            crate::command::workflow_task_set::executability::UnprovenCause,
        ),
    >,
) -> bool {
    accepted_ids.iter().all(|id| {
        causes.get(id).is_some_and(|(_, cause)| {
            cause.class() == crate::command::workflow_host_fault::Class::Transient
        }) || evidence
            .iter()
            .find(|item| &item.acceptance_id == id)
            .is_some_and(|item| item.operational_error.is_none())
    })
}

pub(super) fn verdicts(report: &types::ReverifyReport) -> String {
    report
        .subjects
        .iter()
        .map(|subject| format!("{}={}", subject.task_id, subject.verdict))
        .chain(
            report
                .checks
                .iter()
                .map(|check| format!("{}={}", check.check_id, check.verdict)),
        )
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn context(
    program: PathBuf,
    (project_root, prd_path, task_root): (&Path, &Path, &Path),
    run_dir: &Path,
    prd_digest: String,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> HostCommandResolutionContext {
    HostCommandResolutionContext {
        program,
        project_root: project_root.to_path_buf(),
        prd_path: prd_path.to_path_buf(),
        prd_digest,
        task_root: task_root.to_path_buf(),
        run_staging_root: run_dir.join("host-command-staging"),
        frozen_task_id: None,
        frozen_task_file: None,
        freeze_provider_environment:
            crate::command::workflow_decompose::freeze_provider_environment(env_vars),
        acceptance_environment_allowlist: config
            .workflow
            .acceptance_execution
            .as_ref()
            .map(|policy| policy.environment_allowlist.clone())
            .unwrap_or_default(),
        gate_mode: config.workflow.gate_mode,
    }
}

#[cfg(test)]
#[path = "workflow_decompose_reverify_judge_heal_tests.rs"]
mod tests;
