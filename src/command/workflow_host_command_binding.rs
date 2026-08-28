//! Binding a candidate TASK body to the one frozen subject it belongs to.

use std::path::Path;

use archon_workflow::{HostCommandRequest, WorkflowError, WorkflowResult};

use super::workflow_host_command_catalog::HostCommandResolutionContext;
use super::workflow_host_command_postcondition::read_acceptance_pin;

pub(crate) fn unbound_context(
    base: &HostCommandResolutionContext,
    run_root: &Path,
) -> HostCommandResolutionContext {
    let mut context = base.clone();
    context.run_staging_root = run_root.join("host-command-staging");
    context
}

/// Which frozen subject a candidate body belongs to.
///
/// Exactly one frozen task must accept the candidate as its own file. Zero or
/// several is the author's artifact being wrong, reported as [`SpecInvalid`] so
/// the caller can refuse the candidate and give the author its next attempt.
pub(crate) fn context_for_request(
    base: &HostCommandResolutionContext,
    run_root: &Path,
    request: &HostCommandRequest,
) -> WorkflowResult<HostCommandResolutionContext> {
    let mut context = unbound_context(base, run_root);
    if request.command_id != "land-task-body"
        || (context.frozen_task_id.is_some() && context.frozen_task_file.is_some())
    {
        return Ok(context);
    }
    let candidate = request.stdin.as_deref().ok_or_else(|| {
        WorkflowError::SpecInvalid("land-task-body requires candidate stdin".to_string())
    })?;
    let pin = read_acceptance_pin(&context)?;
    let skeleton = archon_workflow::task_skeleton::validate_full_chain(&context.task_root, &pin)
        .map_err(|error| WorkflowError::SpecInvalid(error.to_string()))?;
    let mut matches = Vec::new();
    for frozen in &skeleton.tasks {
        let path = context.task_root.join(&frozen.file_name);
        if archon_workflow::task_universe::parsing::parse_task_file(&path, candidate).is_ok() {
            matches.push((frozen.task_id.clone(), path));
        }
    }
    if matches.len() != 1 {
        return Err(WorkflowError::SpecInvalid(format!(
            "candidate TASK body binds {} frozen subjects; return exactly one body preserving a frozen task_id and file_name",
            matches.len()
        )));
    }
    let (task_id, task_file) = matches.pop().expect("one candidate subject");
    context.frozen_task_id = Some(task_id);
    context.frozen_task_file = Some(task_file);
    Ok(context)
}
