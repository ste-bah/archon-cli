//! CLI rendering for a generated-V2 restart.
//!
//! The restart itself — reading the persisted host-call manifest, invalidating
//! the V2 result cache for a call or a task and everything downstream of it,
//! and putting the affected stage states back to pending — is
//! [`archon_workflow::v2::restart`]. That is run-directory knowledge, not
//! command knowledge. What is left here is the one thing that could not follow
//! it: the sentence an operator reads, which names the next slash command.

use super::*;

pub(super) use archon_workflow::v2::restart::{
    GeneratedV2RestartTarget, generated_v2_restart_target, invalidate_generated_v2_call,
    invalidate_generated_v2_item,
};

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
