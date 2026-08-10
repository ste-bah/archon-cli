// WorkflowScriptHost: metadata review, summary and event emission.
// One of three inherent `impl WorkflowScriptHost` blocks split out of
// `workflow_live_v2_script_host.rs` to hold the 500-line ceiling.

use super::*;

impl WorkflowScriptHost {
    pub(super) fn generated_decomposed_prd_run(&self) -> bool {
        self.runner.task_universe.is_some()
    }

    pub(super) async fn persist_source_metadata_review(
        &self,
        execution: WorkflowV2CallExecution,
        source_metadata: archon_workflow::v2::source_graph::DynamicWaveSourceMetadata,
        input_hash: String,
        attempt: u32,
    ) -> archon_workflow::WorkflowResult<String> {
        let call_id = execution.call.id.clone();
        let reason = source_metadata.invalid_reason.clone().unwrap_or_else(|| {
            if source_metadata.unresolved_dependencies.is_empty() {
                "dynamic write fanout source metadata was incomplete".to_string()
            } else {
                format!(
                    "dynamic write fanout has unresolved dependencies: {}",
                    source_metadata.unresolved_dependencies.join(", ")
                )
            }
        });
        let result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: reason.clone(),
            data: serde_json::json!({
                "status": "needs_review",
                "outcomes": [],
                "source_metadata_invalid": reason.clone(),
                "unresolved_dependencies": source_metadata.unresolved_dependencies.clone(),
            }),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Review,
                reason.clone(),
            )],
            residual_gaps: vec![WorkflowV2ResidualGap {
                id: format!(
                    "dynamic_wave_source_metadata_{}",
                    sanitize_v2_gap_id(&call_id)
                ),
                description: reason,
                severity: Some("review".to_string()),
            }],
            ..WorkflowV2Result::default()
        };
        let record = WorkflowV2CallRecord::new(
            self.runner.v2_store.run_id(),
            execution.call.clone(),
            attempt,
            input_hash,
            result,
            execution.depends_on,
        )
        .with_source_metadata(
            source_metadata.source_fingerprint.clone(),
            source_metadata.source_task_graph.clone(),
        )
        .with_scaffold_hash(Some(self.scaffold_hash.clone()));
        self.runner.v2_store.save_call_record(&record)?;
        self.update_checkpoint(&record)?;
        self.mark_executed(&record, record.status).await;
        self.emit_call_finished_event(&record);
        result_view_json(&record.result)
    }

    pub(crate) async fn summary(&self) -> WorkflowV2ScriptSummary {
        let acc = self.accumulator.lock().await;
        WorkflowV2ScriptSummary {
            status: acc.status,
            completed: acc.completed,
            executed: acc.executed,
            reused: acc.reused,
            calls: acc.calls.clone(),
            failed_call: acc.failed_call.clone(),
            failed_result_path: acc.failed_result_path.clone(),
            next_action: acc.next_action.clone(),
            script_result: None,
        }
    }

    pub(super) fn emit_call_finished_event(&self, record: &WorkflowV2CallRecord) {
        let result_path = self
            .runner
            .v2_store
            .result_path(&record.call.id)
            .display()
            .to_string();
        self.emit_v2_event(
            match record.status {
                WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => {
                    WorkflowEventKind::StageFailed
                }
                WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview => {
                    WorkflowEventKind::StageStalled
                }
                _ => WorkflowEventKind::StageCompleted,
            },
            serde_json::json!({
                "event": match record.status {
                    WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => "call_failed",
                    WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview => "call_needs_review",
                    _ => "call_finished",
                },
                "call_id": record.call.id.clone(),
                "method": record.call.method.as_str(),
                "status": record.status,
                "result_path": result_path.clone(),
            }),
        );
        self.emit_blocking_gap_events(record, &result_path);
    }

    /// Name every blocking residual gap the record carries in `events.jsonl`.
    ///
    /// Emitted from here, next to the call-finished event, because this is the
    /// one place every newly persisted call record passes through — both
    /// `run_v2_host_call` and `persist_source_metadata_review` reach it. Reuse
    /// (`mark_reused`) deliberately does not: a reused record was persisted
    /// earlier in this same run directory, so its gap events are already in
    /// this same `events.jsonl` and re-emitting them would double-count.
    ///
    /// Emission is best-effort in exactly the way the surrounding event calls
    /// are: a log write must never change a call's outcome.
    pub(super) fn emit_blocking_gap_events(
        &self,
        record: &WorkflowV2CallRecord,
        result_path: &str,
    ) {
        let Ok(events) = archon_workflow::events::blocking_gap_events::build_blocking_gap_events(
            record,
            result_path,
        ) else {
            return;
        };
        for (kind, detail) in events {
            self.emit_v2_event(kind, detail);
        }
    }

    pub(super) fn emit_v2_event(&self, kind: WorkflowEventKind, detail: serde_json::Value) {
        let Ok(seq) = self
            .runner
            .workflow_store
            .next_event_seq(&self.runner.run_id)
        else {
            return;
        };
        let _ = WorkflowEventLog::new(self.runner.workflow_store.clone()).emit(
            &self.runner.run_id,
            seq,
            kind,
            detail,
        );
    }

    pub(crate) fn emit_terminal_status(&self, status: WorkflowV2Status) {
        self.emit_v2_event(
            match status {
                WorkflowV2Status::Accepted | WorkflowV2Status::Noop => {
                    WorkflowEventKind::StageCompleted
                }
                WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => {
                    WorkflowEventKind::StageFailed
                }
                WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview => {
                    WorkflowEventKind::StageStalled
                }
                WorkflowV2Status::Pending | WorkflowV2Status::Running => {
                    WorkflowEventKind::StageStarted
                }
            },
            serde_json::json!({
                "event": "terminal_status",
                "status": status,
            }),
        );
    }
}
