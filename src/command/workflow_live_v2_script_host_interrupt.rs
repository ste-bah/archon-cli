// WorkflowScriptHost: what happens when a pause or cancel kills a call.
//
// Its own module rather than a corner of `..._exec.rs` because the two answer
// different questions — that file is about running a call to a result, this one
// about a call that will never have one — and because an interrupted call has
// to leave the same two traces a finished one does: a record on disk AND an
// event in the run log. Keeping both halves together is what stops the second
// from being forgotten again.
use super::*;

/// Whether `err` is a control decision that killed the call rather than an
/// outcome of the work. `NotificationDelivery` is deliberately absent — see
/// `execute`.
pub(super) fn control_interruption_reason(err: &WorkflowError) -> Option<&'static str> {
    match err {
        WorkflowError::ControlCancelled(_) => Some("cancelled"),
        WorkflowError::ControlPaused(_) => Some("paused"),
        _ => None,
    }
}

impl WorkflowScriptHost {
    /// Persist why a call stopped when a pause or cancel killed it mid-flight.
    ///
    /// Mirrors [`failed_v2_result`]'s shape but claims `NeedsReview`, not
    /// `Failed`: the work did not fail, it was stopped, so the honest statement
    /// is that it did not complete and a human must decide. That status is
    /// outside `is_reusable_status`, so this can never be replayed as a success,
    /// and it takes no `residual_gaps` — an interrupted call establishes no gap.
    pub(super) async fn save_interrupted_call_record(
        &self,
        execution: &WorkflowV2CallExecution,
        reason: &str,
        err: &WorkflowError,
        elapsed: std::time::Duration,
        attempt: u32,
        input_hash: &str,
        source_fingerprint: Option<String>,
        dispatch_generation: Option<u64>,
    ) {
        let call_id = &execution.call.id;
        let elapsed_seconds = elapsed.as_secs();
        let detail = err.to_string();
        let summary = format!(
            "workflow v2 call '{call_id}' was {reason} after {elapsed_seconds}s in flight and produced no result: {detail}"
        );
        let result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: summary.clone(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Blocker,
                summary,
            )],
            data: serde_json::json!({
                "call_id": call_id,
                "interrupted": reason,
                "elapsed_seconds": elapsed_seconds,
                "error": detail,
            }),
            ..WorkflowV2Result::default()
        };
        let record = WorkflowV2CallRecord::new(
            self.runner.v2_store.run_id(),
            execution.call.clone(),
            attempt,
            input_hash.to_string(),
            result,
            execution.depends_on.clone(),
        )
        // No source task graph: it seeds `completed_ids` for a call that did none.
        .with_source_metadata(source_fingerprint, None)
        .with_scaffold_hash(Some(self.scaffold_hash.clone()));
        let persisted = self.runner.workflow_store.with_run_lock(
            &self.runner.run_id,
            |locked| {
                let current = locked.load_state(&self.runner.run_id)?;
                let control_state = matches!(
                    current.status,
                    archon_workflow::RunStatus::Paused
                        | archon_workflow::RunStatus::Cancelled
                );
                if !control_state
                    || dispatch_generation.is_some_and(|generation| {
                        current.generation != generation.saturating_add(1)
                    })
                {
                    return Err(WorkflowError::ControlCancelled(format!(
                        "fixed interrupted call generation {:?} no longer owns control evidence for run {}; current generation/status is {}/{:?}",
                        dispatch_generation,
                        self.runner.run_id,
                        current.generation,
                        current.status
                    )));
                }
                self.runner.v2_store.save_call_record(&record)?;
                let event = crate::command::workflow_decompose_state::project_fixed_call(
                    locked,
                    &self.runner.run_id,
                    &record,
                    crate::command::workflow_decompose_state::FixedCallProjectionKind::Interrupted,
                )?;
                self.emit_call_finished_event(&record);
                Ok(event)
            },
        );
        let event = match persisted {
            Ok(event) => event,
            Err(err) => {
                tracing::warn!(%call_id, reason, %err, "interrupted call evidence not saved");
                return;
            }
        };
        if let Some(event) = event
            && let Err(err) = self.runner.client.ui_sink.emit(event).await
        {
            tracing::warn!(%call_id, reason, %err, "interrupted call UI event not delivered");
        }
    }
}

#[cfg(test)]
#[path = "workflow_live_v2_script_host_interrupt_tests.rs"]
mod workflow_live_v2_script_host_interrupt_tests;
