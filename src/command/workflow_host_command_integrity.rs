use archon_workflow::{WorkflowError, WorkflowResult};

use super::workflow_host_command_catalog::HostCommandResolutionContext;

pub(crate) fn require_launch_prd_unchanged(
    context: &HostCommandResolutionContext,
    command_id: &str,
) -> WorkflowResult<()> {
    if command_id != "freeze-acceptance" {
        return Ok(());
    }
    let (_, current_digest, _) = super::workflow_task_set::validate_prd_input(&context.prd_path)
        .map_err(|error| WorkflowError::SpecInvalid(error.to_string()))?;
    if current_digest != context.prd_digest {
        return Err(WorkflowError::SpecInvalid(
            "fixed acceptance freeze PRD differs from the launch PRD digest".into(),
        ));
    }
    Ok(())
}
