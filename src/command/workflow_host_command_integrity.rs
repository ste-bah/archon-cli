use archon_workflow::{WorkflowError, WorkflowResult};

use super::workflow_host_command_catalog::HostCommandResolutionContext;

pub(crate) fn require_launch_prd_unchanged(
    context: &HostCommandResolutionContext,
    command_id: &str,
) -> WorkflowResult<()> {
    if command_id != "freeze-acceptance" {
        return Ok(());
    }
    let input = (|| -> anyhow::Result<_> {
        #[cfg(test)]
        super::workflow_host_fault_test_hooks::fire(
            super::workflow_host_fault_test_hooks::Point::LaunchInput,
            &context.prd_path,
        )?;
        super::workflow_task_set::validate_prd_input(&context.prd_path)
    })();
    let (_, current_digest, _) = input.map_err(|error| {
        if super::workflow_host_fault::classify(&error)
            == super::workflow_host_fault::Class::Transient
        {
            super::workflow_host_fault::workflow(
                error,
                format!("reading launch input {}", context.prd_path.display()),
            )
        } else {
            WorkflowError::SpecInvalid(super::workflow_host_fault::diagnostic(&error))
        }
    })?;
    if current_digest != context.prd_digest {
        return Err(WorkflowError::SpecInvalid(format!(
            "fixed acceptance freeze PRD differs from the launch PRD digest; restore {} to the recorded digest {}",
            context.prd_path.display(),
            context.prd_digest
        )));
    }
    Ok(())
}
