use super::*;

impl WorkflowScriptHost {
    /// Emit the ordinary session's reuse notice; fixed decomposition records
    /// its own stage projection and keeps this UI event silent.
    pub(super) async fn emit_reuse_status(
        &self,
        execution: &WorkflowV2CallExecution,
    ) -> archon_workflow::WorkflowResult<()> {
        if self.fixed_decomposition_state_present() {
            return Ok(());
        }
        self.runner
            .client
            .ui_sink
            .emit(WorkflowUiEvent::Text(format!(
                "Workflow V2 script call reused: {} via w.{}\n",
                execution.call.id,
                execution.call.method.as_str()
            )))
            .await
            .map_err(|error| {
                WorkflowError::NotificationDelivery(format!(
                    "workflow call reuse status delivery failed: run_id={} stage_id={} status=reused: {error}",
                    self.runner.run_id, execution.call.id
                ))
            })
    }
}
