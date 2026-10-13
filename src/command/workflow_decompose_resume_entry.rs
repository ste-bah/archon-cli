use super::*;

pub(crate) async fn resume_fixed_decomposition_with_factory(
    cwd: &Path,
    run_id: &str,
    yes: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    factory: &dyn WorkflowLlmClientFactory,
) -> Result<String> {
    super::resume_fixed_decomposition_with_factory_and_sink(
        cwd,
        run_id,
        yes,
        config,
        env_vars,
        factory,
        crate::command::workflow_decompose_progress::DecompositionCliUiSink::shared(),
        None,
        None,
    )
    .await
}

/// A resume whose cancellation was requested before provider construction
/// ends here, cancelling the run unless it already reached a terminal status.
pub(super) fn cancel_if_requested(
    store: &WorkflowStore,
    run_id: &str,
    cancellation_requested: Option<&std::sync::atomic::AtomicBool>,
) -> Result<()> {
    if cancellation_requested
        .is_some_and(|requested| requested.load(std::sync::atomic::Ordering::SeqCst))
    {
        let current = store.load_state(run_id)?;
        if !matches!(
            current.status,
            archon_workflow::RunStatus::Completed
                | archon_workflow::RunStatus::Cancelled
                | archon_workflow::RunStatus::Failed
                | archon_workflow::RunStatus::Blocked
        ) {
            archon_workflow::LifecycleController::new(store.clone())
                .apply(run_id, archon_workflow::LifecycleAction::Cancel)?;
        }
        return Err(anyhow!(
            "fixed decomposition resume cancelled before provider construction"
        ));
    }
    Ok(())
}
