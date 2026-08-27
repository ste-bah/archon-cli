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
        let (mut options, write_mode) = parse_script_options(&request.options)?;
        if method == WorkflowV2HostMethod::HostCommand {
            options.host_command = Some(parse_host_command_request(&request)?);
        }
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

    pub(super) fn fixed_decomposition_state_present(&self) -> bool {
        self.runner
            .workflow_store
            .run_dir(&self.runner.run_id)
            .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH)
            .exists()
    }

    pub(super) async fn project_fixed_call_and_emit(
        &self,
        record: &WorkflowV2CallRecord,
        kind: crate::command::workflow_decompose_state::FixedCallProjectionKind,
    ) -> archon_workflow::WorkflowResult<bool> {
        let event = crate::command::workflow_decompose_state::project_fixed_call(
            &self.runner.workflow_store,
            &self.runner.run_id,
            record,
            kind,
        )?;
        let Some(event) = event else {
            return Ok(false);
        };
        self.runner
            .client
            .ui_sink
            .emit(event)
            .await
            .map_err(|error| {
                WorkflowError::NotificationDelivery(format!(
                    "fixed decomposition progress delivery failed after durable log flush: {error}"
                ))
            })?;
        Ok(true)
    }

    pub(super) async fn persist_fixed_call_started(
        &self,
        execution: &WorkflowV2CallExecution,
        attempt: u32,
        input_hash: &str,
    ) -> archon_workflow::WorkflowResult<()> {
        if !self.fixed_decomposition_state_present() {
            return Ok(());
        }
        let mut call = execution.call.clone();
        call.options.task = None;
        call.options.source = None;
        call.options.extra.clear();
        if let Some(request) = &mut call.options.host_command {
            request.stdin = None;
        }
        let mut result = WorkflowV2Result::default();
        result.status = WorkflowV2Status::Running;
        result.summary = "fixed decomposition call in flight".to_string();
        let record = WorkflowV2CallRecord::new(
            self.runner.v2_store.run_id(),
            call,
            attempt,
            input_hash.to_string(),
            result,
            execution.depends_on.clone(),
        )
        .with_scaffold_hash(Some(self.scaffold_hash.clone()));
        self.runner.v2_store.save_call_record(&record)?;
        self.project_fixed_call_and_emit(
            &record,
            crate::command::workflow_decompose_state::FixedCallProjectionKind::Started,
        )
        .await?;
        Ok(())
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
        self.project_fixed_call_and_emit(
            record,
            crate::command::workflow_decompose_state::FixedCallProjectionKind::Reused,
        )
        .await?;
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

    pub(super) async fn mark_executed(
        &self,
        record: &WorkflowV2CallRecord,
        status: WorkflowV2Status,
    ) {
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

    pub(crate) async fn mark_script_failure(&self, error: &str) -> WorkflowV2ScriptSummary {
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
        self.summary().await
    }
}
