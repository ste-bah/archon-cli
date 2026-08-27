//! Typed HostCommand result adapter inside the ordinary persisted script-call path.

use super::*;

impl WorkflowScriptHost {
    pub(super) async fn execute_host_command(
        &self,
        execution: &WorkflowV2CallExecution,
    ) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
        let request = execution.call.options.host_command.clone().ok_or_else(|| {
            WorkflowError::SpecInvalid("HostCommand call is missing its typed request".to_string())
        })?;
        let executor = self.runner.host_command_executor.as_ref().ok_or_else(|| {
            WorkflowError::PolicyDenied(
                "HostCommand is available only to a trusted fixed workflow run".to_string(),
            )
        })?;
        let command_id = request.command_id.clone();
        let outcome = executor.execute(request).await?;
        let status = if outcome.reusable() {
            WorkflowV2Status::Accepted
        } else {
            WorkflowV2Status::NeedsReview
        };
        let mut result = WorkflowV2Result {
            status,
            summary: format!(
                "host command '{}' completed with exit {:?}",
                execution.call.id, outcome.exit_code
            ),
            data: serde_json::to_value(&outcome)?,
            ..WorkflowV2Result::default()
        };
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            format!(
                "trusted host command process completed: exit={:?}, stdout_bytes={}, stderr_bytes={}",
                outcome.exit_code, outcome.stdout_bytes, outcome.stderr_bytes
            ),
        ));
        result.commands_run.push(archon_workflow::WorkflowV2CommandRecord {
            kind: archon_workflow::WorkflowV2CommandKind::Inspect,
            command: format!("hostCommand:{command_id}"),
            status: if outcome.exit_code == Some(0) {
                archon_workflow::WorkflowV2CommandStatus::Succeeded
            } else {
                archon_workflow::WorkflowV2CommandStatus::Failed
            },
            exit_code: outcome.exit_code,
            output_summary: format!(
                "trusted symbolic capability completed with {} stdout bytes and {} stderr bytes",
                outcome.stdout_bytes, outcome.stderr_bytes
            ),
        });
        Ok(result)
    }
}
