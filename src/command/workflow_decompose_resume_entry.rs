use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn resume_fixed_decomposition_with_factory_and_sink(
    cwd: &Path,
    run_id: &str,
    yes: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    factory: &dyn WorkflowLlmClientFactory,
    ui_sink: archon_workflow::SharedWorkflowUiSink,
    interactive_owner: Option<&str>,
    cancellation_requested: Option<&std::sync::atomic::AtomicBool>,
) -> Result<String> {
    super::resume_fixed_decomposition_at_binary_revision(
        cwd,
        run_id,
        yes,
        config,
        env_vars,
        factory,
        ui_sink,
        interactive_owner,
        cancellation_requested,
        &crate::command::workflow_test_seams::binary_revision(),
    )
    .await
}

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
/// Admission may begin paused; only a new control decision interrupts recovery.
pub(super) fn recovery_control(
    store: &WorkflowStore,
    admitted: &archon_workflow::WorkflowRun,
    cancellation_requested: Option<&std::sync::atomic::AtomicBool>,
) -> Result<()> {
    cancel_if_requested(store, &admitted.id, cancellation_requested)?;
    let current = store.load_state(&admitted.id)?;
    if current.generation != admitted.generation || current.status != admitted.status {
        let text = format!(
            "run {} changed control during recorded-child recovery",
            admitted.id
        );
        return Err(if current.status == archon_workflow::RunStatus::Paused {
            archon_workflow::WorkflowError::ControlPaused(text)
        } else {
            archon_workflow::WorkflowError::ControlCancelled(text)
        }
        .into());
    }
    Ok(())
}

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
