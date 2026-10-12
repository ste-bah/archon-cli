//! Host-owned frontmatter stamping for one bound body landing.

use archon_workflow::{HostCommandRequest, WorkflowError, WorkflowResult};

use super::super::workflow_host_command_catalog::HostCommandResolutionContext;

pub(super) fn stamp_request(
    request: &mut HostCommandRequest,
    context: &HostCommandResolutionContext,
) -> WorkflowResult<Vec<String>> {
    if request.command_id != "land-task-body" {
        return Ok(Vec::new());
    }
    let pin = super::super::workflow_host_command_postcondition::read_acceptance_pin(context)?;
    let skeleton = archon_workflow::task_skeleton::validate_full_chain(&context.task_root, &pin)
        .map_err(|error| WorkflowError::SpecInvalid(error.to_string()))?;
    let task_id = context.frozen_task_id.as_deref().ok_or_else(|| {
        WorkflowError::SpecInvalid("land-task-body has no host-bound frozen task id".into())
    })?;
    let frozen = skeleton
        .tasks
        .iter()
        .find(|task| task.task_id == task_id)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "bound task {task_id} is absent from frozen skeleton"
            ))
        })?;
    let candidate = request.stdin.as_deref().ok_or_else(|| {
        WorkflowError::SpecInvalid("land-task-body requires candidate stdin".into())
    })?;
    let path = context.frozen_task_file.as_ref().ok_or_else(|| {
        WorkflowError::SpecInvalid("land-task-body has no host-bound frozen task file".into())
    })?;
    let subjects = [(task_id, path.as_path())];
    let anchor = crate::command::topology_lint::task_file_anchor(candidate, &subjects)
        .map_err(|_| {
            WorkflowError::SpecInvalid("bound task body contains multiple task files".into())
        })?
        .ok_or_else(|| {
            WorkflowError::SpecInvalid("bound task body has no task file anchor".into())
        })?;
    let (stamped, notes) = super::super::workflow_host_command_binding::stamp_frozen_frontmatter(
        &candidate[anchor.opener..],
        frozen,
    )?;
    request.stdin = Some(format!("{}{}", &candidate[..anchor.opener], stamped));
    Ok(notes)
}
