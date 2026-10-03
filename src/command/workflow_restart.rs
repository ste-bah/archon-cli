//! CLI rendering for a generated-V2 restart.
//!
//! The restart itself — reading the persisted host-call manifest, invalidating
//! the V2 result cache for a call or a task and everything downstream of it,
//! and putting the affected stage states back to pending — is
//! [`archon_workflow::v2::restart`]. That is run-directory knowledge, not
//! command knowledge. What is left here is the one thing that could not follow
//! it: the sentence an operator reads, which names the next slash command.

use super::*;

// The slash surface restarts through `LifecycleController::apply_restart`
// (Issue-267); these stay reachable for the tests of the restart cache.
#[cfg(test)]
pub(super) use archon_workflow::v2::restart::{
    GeneratedV2RestartTarget, generated_v2_restart_target, invalidate_generated_v2_call,
    invalidate_generated_v2_item,
};

/// Runs one restart of `run_id` while no live executor can run it (Issue
/// 256/267). The restart holds the run's executor lease for its whole
/// duration, taken exactly as `begin_execution` takes it (the file is
/// created when missing). A live executor holds that lease, so the restart
/// is refused; and an executor that tries to start during the restart is
/// refused in turn. There is no window between a check and the restart.
/// A live session that holds no lease (a run that is not a fixed
/// decomposition) is stopped by the restart epoch the restart moves on.
pub(super) fn with_restart_lease<T>(
    store: &WorkflowStore,
    run_id: &str,
    restart: impl FnOnce() -> Result<T>,
) -> Result<T> {
    // The run must exist before its lease file may be created.
    store.load_state(run_id)?;
    let _lease = crate::command::workflow_executor_lease::acquire(&store.run_dir(run_id), run_id)
        .map_err(|error| {
            anyhow!(
                "workflow restart of {run_id} refused: {error}; pause the run and wait for its executor to exit, then restart"
            )
        })?;
    restart()
}

pub(super) fn restart_generated_v2_task_workflow(
    store: &WorkflowStore,
    run: &WorkflowRun,
    task_id: &str,
) -> Result<Option<String>> {
    let Some(invalidation) =
        archon_workflow::v2::restart::restart_generated_v2_task(store, run, task_id)?
    else {
        return Ok(None);
    };
    Ok(Some(format_generated_v2_task_restart(
        &run.id,
        task_id,
        &invalidation,
    )))
}

fn format_generated_v2_task_restart(
    run_id: &str,
    requested_task_id: &str,
    invalidation: &WorkflowV2TaskInvalidation,
) -> String {
    format!(
        "Workflow generated V2 task restart prepared: task {requested_task_id} resolved to {}.\naffected_tasks: {}\ninvalidated_calls: {}\ndeleted_branch_outcomes: {}\nNext: /workflow continue {run_id}\n",
        invalidation.requested_task_id,
        invalidation.affected_task_ids.join(", "),
        invalidation.invalidated_call_ids.join(", "),
        invalidation.deleted_branch_outcomes.len(),
    )
}

#[cfg(test)]
#[path = "workflow_restart_tests.rs"]
mod tests;
