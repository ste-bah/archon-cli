//! Typed HostCommand result adapter inside the ordinary persisted script-call path.

use super::*;

impl WorkflowScriptHost {
    /// `data` with what the host command `call` ran under: its limits
    /// (Issue 358), so a resume replays an outcome a limit cut short only
    /// under the same limits, and the logic version and source digest that
    /// judged it (Issue 361), so a resume replays a verdict only under the
    /// same logic. Other calls, and executors that apply none, are unchanged.
    pub(super) fn stamp_host_outcome(
        &self,
        call: &archon_workflow::WorkflowV2HostCall,
        mut data: serde_json::Value,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        if call.method != WorkflowV2HostMethod::HostCommand {
            return Ok(data);
        }
        let (Some(executor), Some(request)) = (
            self.runner.host_command_executor.as_ref(),
            call.options.host_command.as_ref(),
        ) else {
            return Ok(data);
        };
        let limits = executor.limits_fingerprint(request)?;
        let logic = executor.logic_version(request)?;
        let digest = executor.logic_digest(request)?;
        if let Some(map) = data.as_object_mut() {
            if let Some(stamp) = limits {
                map.insert(
                    crate::command::workflow_host_command_exec::identity::LIMITS_FINGERPRINT.into(),
                    stamp,
                );
            }
            if let Some(version) = logic {
                map.insert(
                    crate::command::workflow_host_command_logic::LOGIC_VERSION_STAMP.into(),
                    serde_json::json!(version),
                );
            }
            if let Some(digest) = digest {
                map.insert(
                    crate::command::workflow_host_command_logic::LOGIC_DIGEST_STAMP.into(),
                    serde_json::json!(digest),
                );
            }
        }
        Ok(data)
    }

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
            executor.execute(request, expected_generation),
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
            data: self.stamp_host_outcome(&execution.call, serde_json::to_value(&outcome)?)?,
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
