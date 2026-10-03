//! The executor lease of a generic V2 run (Issue 252).
//!
//! The same kernel lock a fixed decomposition holds
//! (`workflow_executor_lease`), taken by every launch and resume and held for
//! the whole executor lifetime, including detached blocking scripts and their
//! runtimes, so two processes never execute one run. A live
//! holder refuses a second resume with its pid, before anything in the run
//! directory changes. A resume that finds the run `Running` while the lease
//! is free proves its executor dead: the recovery is recorded as a
//! `stale_owner_recovered` event and the run moves to `Paused`, the state the
//! resume path accepts.

use super::*;
use crate::command::workflow_executor_lease::ExecutionLease;

/// The lease of a run this process is about to execute.
pub(super) fn take(store: &WorkflowStore, run_id: &str) -> Result<Arc<ExecutionLease>> {
    crate::command::workflow_task_root_reclaim::begin_execution(store, run_id).map(Arc::new)
}

/// The lease of a run this process is about to resume, with a dead
/// executor's `Running` state recovered first.
pub(super) async fn take_for_resume(
    store: &WorkflowStore,
    run_id: &str,
    ui_sink: &SharedWorkflowUiSink,
) -> Result<Arc<ExecutionLease>> {
    let lease = take(store, run_id)?;
    if store.load_state(run_id)?.status == RunStatus::Running {
        // This process holds the lease, so any host-command group left here
        // is the dead executor's; one still running refuses the recovery.
        let ended = crate::command::workflow_host_command_groups::require_no_running_groups(
            &store.run_dir(run_id),
            run_id,
        )?;
        if let Some(recovery) =
            crate::command::workflow_decompose_stale_owner::recover_dead_generic_owner(
                store, run_id, &lease, &ended,
            )?
        {
            ui_sink
                .emit(archon_workflow::WorkflowUiEvent::Text(
                    recovery.summary(run_id),
                ))
                .await
                .map_err(|error| anyhow::anyhow!("reporting stale owner recovery: {error}"))?;
        }
    }
    Ok(lease)
}

#[cfg(test)]
#[path = "workflow_live_v2_run_lease_tests.rs"]
mod tests;
