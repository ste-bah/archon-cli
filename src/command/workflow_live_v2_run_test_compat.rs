use super::*;

#[cfg(test)]
pub(crate) async fn run_generated_v2_workflow(
    cwd: &Path,
    store: &WorkflowStore,
    plan: WorkflowScriptPlan,
    task: String,
    llm: Arc<dyn WorkflowLlmClient>,
    ui_sink: SharedWorkflowUiSink,
    agent_names: Vec<String>,
    approval_mode: LiveApprovalMode,
    workspace_boundary_supported: bool,
    script_lifecycle: bool,
    learning: &archon_core::config::LearningConfig,
) -> Result<String> {
    run_generated_v2_workflow_with_limits(
        cwd,
        store,
        plan,
        task,
        llm,
        ui_sink,
        agent_names,
        approval_mode,
        workspace_boundary_supported,
        script_lifecycle,
        learning,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        Arc::new(archon_core::config::ArchonConfig::default()),
    )
    .await
}

#[cfg(test)]
pub(crate) async fn resume_generated_v2_workflow(
    cwd: &Path,
    store: &WorkflowStore,
    run_id: &str,
    llm: Arc<dyn WorkflowLlmClient>,
    ui_sink: SharedWorkflowUiSink,
    agent_names: Vec<String>,
    approval_mode: LiveApprovalMode,
    workspace_boundary_supported: bool,
    learning: &archon_core::config::LearningConfig,
) -> Result<Option<String>> {
    resume_generated_v2_workflow_with_limits(
        cwd,
        store,
        run_id,
        llm,
        ui_sink,
        agent_names,
        approval_mode,
        workspace_boundary_supported,
        learning,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        Arc::new(archon_core::config::ArchonConfig::default()),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) async fn execute_generated_v2_run(
    store: &WorkflowStore,
    run: WorkflowRun,
    plan: WorkflowScriptPlan,
    task: String,
    llm: Arc<dyn WorkflowLlmClient>,
    ui_sink: SharedWorkflowUiSink,
    agent_names: Vec<String>,
    workspace_boundary_supported: bool,
    adopt_accepted_cache: bool,
    executor_lease: Arc<crate::command::workflow_executor_lease::ExecutionLease>,
) -> Result<String> {
    execute_generated_v2_run_with_limits(
        store,
        run,
        plan,
        task,
        llm,
        ui_sink,
        agent_names,
        workspace_boundary_supported,
        adopt_accepted_cache,
        executor_lease,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        Arc::new(archon_core::config::ArchonConfig::default()),
    )
    .await
}
