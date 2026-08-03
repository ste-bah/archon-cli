// WorkflowScriptHost: execution records and checkpoint state.
// One of three inherent `impl WorkflowScriptHost` blocks split out of
// `workflow_live_v2_script_host.rs` to hold the 500-line ceiling.

use super::*;

impl WorkflowScriptHost {
    pub(super) fn execution_from_request(
        &self,
        method: &str,
        request: ScriptHostRequest,
    ) -> archon_workflow::WorkflowResult<WorkflowV2CallExecution> {
        let method = WorkflowV2HostMethod::parse(method).ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "workflow.js used unsupported host method w.{method}"
            ))
        })?;
        let (options, write_mode) = parse_script_options(&request.options)?;
        if method == WorkflowV2HostMethod::Implementation && write_mode.is_none() {
            return Err(WorkflowError::SpecInvalid(format!(
                "w.implementation('{}') requires explicit write mode serial, coordinated, or worktree",
                request.id
            )));
        }
        let mut input = serde_json::json!({
            "objective": self.runner.task.clone(),
            "call_id": request.id.clone(),
            "method": method.as_str(),
            "write_mode": write_mode,
            "options": request.options,
        });
        if let Some(source) = request.source {
            input["source_data"] = source;
        }
        let has_explicit_source = input.get("source_data").is_some();
        if let Some(inputs) = input
            .get("options")
            .and_then(|options| options.get("inputs"))
            .cloned()
        {
            input["inputs"] = inputs.clone();
            if !has_explicit_source {
                input["source_data"] = inputs;
            }
        }
        if let Some(source) = options.source.as_deref() {
            input["source"] = serde_json::Value::String(source.to_string());
        }
        Ok(WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: request.id,
                method,
                write_mode,
                options,
            },
            input,
            depends_on: Vec::new(),
        })
    }

    pub(super) fn update_checkpoint(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<()> {
        let mut checkpoint = self
            .runner
            .v2_store
            .load_checkpoint()?
            .unwrap_or_else(WorkflowV2Checkpoint::default);
        if is_reusable_status(record.status) {
            checkpoint.mark_completed(&record.call.id);
        } else {
            checkpoint.remove_completed_call(&record.call.id);
        }
        self.runner.v2_store.save_checkpoint(&checkpoint)
    }

    pub(super) async fn mark_reused(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<()> {
        let mut checkpoint = self
            .runner
            .v2_store
            .load_checkpoint()?
            .unwrap_or_else(WorkflowV2Checkpoint::default);
        checkpoint.mark_completed(&record.call.id);
        self.runner.v2_store.save_checkpoint(&checkpoint)?;
        let mut acc = self.accumulator.lock().await;
        acc.status = merge_v2_status(acc.status, record.status);
        acc.reused += 1;
        acc.completed += 1;
        acc.calls.push(record.call.clone());
        drop(acc);
        self.emit_v2_event(
            WorkflowEventKind::StageCompleted,
            serde_json::json!({
                "event": "call_reused",
                "call_id": record.call.id.clone(),
                "method": record.call.method.as_str(),
                "status": record.status,
                "result_path": self.runner.v2_store.result_path(&record.call.id).display().to_string(),
            }),
        );
        Ok(())
    }

    pub(super) async fn mark_executed(&self, record: &WorkflowV2CallRecord, status: WorkflowV2Status) {
        let mut acc = self.accumulator.lock().await;
        // A final report is the script speaking for the whole run: its status
        // overrides accumulated call severities so script-recovered failures
        // do not doom an otherwise accepted run.
        if record.call.method == WorkflowV2HostMethod::FinalReport {
            acc.status = status;
        } else {
            acc.status =
                merge_v2_status(acc.status, run_terminal_status_contribution(record, status));
        }
        acc.executed += 1;
        if is_reusable_status(status) {
            acc.completed += 1;
        }
        acc.calls.push(record.call.clone());
    }

    pub(super) async fn mark_terminal(
        &self,
        record: &WorkflowV2CallRecord,
        result_path: String,
        next_action: String,
    ) {
        let mut acc = self.accumulator.lock().await;
        if record.call.method == WorkflowV2HostMethod::FinalReport {
            acc.status = record.status;
        } else {
            acc.status = merge_v2_status(
                acc.status,
                run_terminal_status_contribution(record, record.status),
            );
        }
        acc.failed_call = Some(record.call.id.clone());
        acc.failed_result_path = Some(result_path);
        acc.next_action = Some(next_action);
    }

    pub(crate) async fn mark_script_failure(
        &self,
        error: &str,
        emit_terminal_status: bool,
    ) -> WorkflowV2ScriptSummary {
        let next_action =
            "fix the workflow.js/runtime error, then resume or start a fresh workflow".to_string();
        let mut acc = self.accumulator.lock().await;
        acc.status = merge_v2_status(acc.status, WorkflowV2Status::Failed);
        acc.failed_call = Some("workflow.js".to_string());
        acc.failed_result_path = None;
        acc.next_action = Some(next_action.clone());
        drop(acc);
        self.emit_v2_event(
            WorkflowEventKind::StageFailed,
            serde_json::json!({
                "event": "script_stopped",
                "call_id": "workflow.js",
                "method": "script",
                "status": WorkflowV2Status::Failed,
                "error": error,
                "next_action": next_action,
            }),
        );
        if emit_terminal_status {
            self.emit_terminal_status(WorkflowV2Status::Failed);
        }
        self.summary().await
    }

}
