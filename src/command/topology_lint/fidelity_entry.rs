//! Entry points capture their site before any asynchronous critic work.
use super::*;

#[cfg(test)]
pub(crate) async fn evaluate_lint_with_fidelity(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
    client: Result<Arc<dyn WorkflowLlmClient>>,
    waivers: &[ObligationWaiver],
) -> Result<GateEvaluation> {
    let site = crate::command::acceptance_check_policy::command_site_path(cwd)?;
    evaluate_lint_with_fidelity_at_path(cwd, source, mode, client, waivers, &site).await
}

pub(crate) async fn evaluate_lint_with_fidelity_at_path(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
    client: Result<Arc<dyn WorkflowLlmClient>>,
    waivers: &[ObligationWaiver],
    site: &std::ffi::OsStr,
) -> Result<GateEvaluation> {
    let resume = FreezeResume::none();
    super::evaluate_lint_with_fidelity_resumable_at_path(
        cwd, source, mode, client, waivers, &resume, site,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn evaluate_lint_with_fidelity_resumable(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
    client: Result<Arc<dyn WorkflowLlmClient>>,
    waivers: &[ObligationWaiver],
    resume: &FreezeResume,
) -> Result<GateEvaluation> {
    let site = crate::command::acceptance_check_policy::command_site_path(cwd)?;
    super::evaluate_lint_with_fidelity_resumable_at_path(
        cwd, source, mode, client, waivers, resume, &site,
    )
    .await
}
