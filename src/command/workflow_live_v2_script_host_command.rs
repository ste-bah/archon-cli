//! Typed HostCommand result adapter inside the ordinary persisted script-call path.

use super::*;

impl WorkflowScriptHost {
    /// Read-only admission and replay observations precede any child or call record.
    pub(super) async fn observe_host_metadata<T>(
        &self,
        capability: &str,
        mut read: impl FnMut() -> archon_workflow::WorkflowResult<T>,
    ) -> archon_workflow::WorkflowResult<T> {
        let generation = self.owned_generation()?;
        crate::command::workflow_host_retry::retry(
            capability,
            (
                &self.runner.workflow_store,
                &self.runner.run_id,
                generation,
                None,
            ),
            || std::future::ready(read().map_err(anyhow::Error::new)),
        )
        .await
        .map_err(|error| {
            error.downcast::<WorkflowError>().unwrap_or_else(|error| {
                crate::command::workflow_host_fault::workflow(error, capability.into())
            })
        })
    }

    /// `data` with what the host command `call` ran under: its limits
    /// (Issue 358), so a resume replays an outcome a limit cut short only
    /// under the same limits, and the logic version, source digest and build
    /// that judged it (Issue 361), so a resume replays a verdict only under
    /// the same logic. Other calls, and executors that apply none, are unchanged.
    pub(super) async fn execute_host_command(
        &self,
        execution: &WorkflowV2CallExecution,
        expected_generation: Option<u64>,
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
        // Batch G: a host command runs outside any agent boundary; one that
        // changed the project's acceptance inputs is restored and fails.
        let run_root = self.runner.v2_store.run_root().to_path_buf();
        let label = format!("host command {command_id} ({})", execution.call.id);
        let (outcome, violation) = archon_workflow::write_coordinator::input_tripwire::watch(
            Some(&run_root),
            &label,
            executor.execute_for_call(request, expected_generation, &execution.call.id),
        )
        .await;
        let outcome = outcome?;
        let status = if violation.is_some() {
            WorkflowV2Status::Failed
        } else if outcome.reusable() {
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
            data: crate::command::workflow_host_command_outcome_stamp::stamp_host_outcome(
                self.runner.host_command_executor.as_deref(),
                &execution.call,
                serde_json::to_value(&outcome)?,
            )?,
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
            pre_existing: false,
        });
        if let Some(violation) = violation {
            result.summary = violation.message();
            result
                .residual_gaps
                .push(archon_workflow::WorkflowV2ResidualGap {
                    id: format!(
                        "environment-violation-{}",
                        archon_workflow::v2::script::sanitize_v2_gap_id(&execution.call.id)
                    ),
                    description: violation.message(),
                    severity: Some("high".to_string()),
                });
        }
        Ok(result)
    }
}
