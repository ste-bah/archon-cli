use super::*;

#[cfg(test)]
pub(crate) async fn prepare_acceptance_freeze(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    client: Arc<dyn WorkflowLlmClient>,
) -> Result<PreparedAcceptanceFreeze> {
    prepare_acceptance_freeze_with_limits(
        project_root,
        tasks_root,
        prd_path,
        mode,
        client,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
}

/// The unstaged freeze of a candidate: unlimited, nothing saved.
#[cfg(test)]
pub(crate) async fn prepare_acceptance_freeze_from_candidate(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    candidate: Vec<u8>,
    client: Arc<dyn WorkflowLlmClient>,
) -> Result<PreparedAcceptanceFreeze> {
    prepare_acceptance_freeze_from_candidate_with_limits(
        project_root,
        tasks_root,
        prd_path,
        mode,
        candidate,
        client,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
}

#[cfg(test)]
pub(crate) async fn prepare_acceptance_freeze_resumable(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    candidate: Vec<u8>,
    client: Arc<dyn WorkflowLlmClient>,
    resume: &crate::command::workflow_freeze_budget::FreezeResume,
) -> Result<PreparedAcceptanceFreeze> {
    prepare_acceptance_freeze_resumable_with_limits(
        project_root,
        tasks_root,
        prd_path,
        mode,
        candidate,
        client,
        resume,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
}
