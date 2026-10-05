// WorkflowScriptHost: what happens when a pause or cancel kills a call.
//
// Its own module rather than a corner of `..._exec.rs` because the two answer
// different questions — that file is about running a call to a result, this one
// about a call that will never have one — and because an interrupted call has
// to leave the same two traces a finished one does: a record on disk AND an
// event in the run log. Keeping both halves together is what stops the second
// from being forgotten again.
use super::*;

/// The reason an interrupted record gives when the call ran and its status
/// could not be delivered (Issue-213 C5). Not a control decision, so its
/// record is written without the paused/cancelled ownership check.
pub(super) const NOTIFICATION_DELIVERY_REASON: &str = "notification_delivery_failed";

/// The reason an interrupted record gives when a host fault (I/O, damaged
/// store) stopped a script that stops on one (Issue 324).
pub(super) const HOST_FAULT_REASON: &str = "host_infrastructure_fault";

impl WorkflowScriptHost {
    /// Issue 324: does `err` stop this script rather than become a value?
    pub(super) fn stops_on_host_fault(&self, err: &WorkflowError) -> bool {
        self.runner.stops_on_host_fault
            && archon_workflow::v2::host_fault::is_host_infrastructure_fault(err)
    }
}

/// The reason a pending call's record gives when a host terminal stop ended
/// the script before the call settled (Issue-285).
pub(super) const TERMINAL_HOST_STOP_REASON: &str = "terminal_host_stop";

/// Whether `err` is a control decision that killed the call rather than an
/// outcome of the work. `NotificationDelivery` is deliberately absent — see
/// `execute`, which records it under [`NOTIFICATION_DELIVERY_REASON`].
pub(super) fn control_interruption_reason(err: &WorkflowError) -> Option<&'static str> {
    match err {
        WorkflowError::ControlCancelled(_) => Some("cancelled"),
        WorkflowError::ControlPaused(_) => Some("paused"),
        _ => None,
    }
}

impl WorkflowScriptHost {
    /// Issue 303: the started record was saved, then publishing it failed, so
    /// the call was never dispatched. Close the record as interrupted now, as
    /// a failed dispatch delivery is, so no `Running` record outlives this
    /// executor; nothing ran, so the next attempt dispatches the call afresh.
    /// A record that cannot be closed keeps its ownership for a terminal stop,
    /// and the next start closes what a crash left (`record_orphaned_calls`).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn close_unstarted_call(
        &self,
        execution: &WorkflowV2CallExecution,
        attempt: u32,
        input_hash: &str,
        source_fingerprint: Option<String>,
        generation: Option<u64>,
        started: std::time::Instant,
        err: &WorkflowError,
    ) {
        let id = &execution.call.id;
        let saved = self
            .runner
            .v2_store
            .load_call_record(id)
            .is_ok_and(|record| {
                record.is_some_and(|record| {
                    record.status == WorkflowV2Status::Running && record.attempt == attempt
                })
            });
        if !saved {
            self.forget_unwritten_pending_call(id);
            return;
        }
        let reason = if matches!(err, WorkflowError::NotificationDelivery(_)) {
            NOTIFICATION_DELIVERY_REASON
        } else {
            super::workflow_live_v2_script_host_inflight::UNSTARTED_REASON
        };
        if let Err(save_err) = self
            .save_interrupted_call_record(
                execution,
                reason,
                err,
                started.elapsed(),
                attempt,
                input_hash,
                source_fingerprint,
                generation,
            )
            .await
        {
            tracing::warn!(call_id = %id, %save_err, "unstarted call record not closed");
            return;
        }
        // A delivery failure already decides the outcome (the bridge records
        // it); any other error reaches the script, which may catch it, so the
        // run must not end as if this call had answered. Not counted as
        // executed: it never ran.
        let mut acc = self.accumulator.lock().await;
        if !acc.terminal_locked() {
            acc.status = merge_v2_status(acc.status, WorkflowV2Status::NeedsReview);
        }
        drop(acc);
        if let Err(stage_err) = self.with_owned_run_lock(|locked| {
            super::workflow_live_v2_script_host_inflight::settle_interrupted_stage(
                locked,
                &self.runner.run_id,
                id,
            )
        }) {
            tracing::warn!(call_id = %id, %stage_err, "unstarted call stage not settled");
        }
    }

    /// Persist why a call stopped when a pause or cancel killed it mid-flight.
    ///
    /// Mirrors [`failed_v2_result`]'s shape but claims `NeedsReview`, not
    /// `Failed`: the work did not fail, it was stopped, so the honest statement
    /// is that it did not complete and a human must decide. That status is
    /// outside `is_reusable_status`, so this can never be replayed as a success,
    /// and it takes no `residual_gaps` — an interrupted call establishes no gap.
    pub(in super::super) async fn save_interrupted_call_record(
        &self,
        execution: &WorkflowV2CallExecution,
        reason: &str,
        err: &WorkflowError,
        elapsed: std::time::Duration,
        attempt: u32,
        input_hash: &str,
        source_fingerprint: Option<String>,
        dispatch_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<()> {
        let call_id = &execution.call.id;
        let elapsed_seconds = elapsed.as_secs();
        let detail = err.to_string();
        // Issue-213 C5 / #215: what the call's sessions had been doing, and
        // which sessions they were.
        let sessions = self.take_call_sessions(call_id);
        let progress = interruption_progress(&sessions);
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
            data: {
                let mut data = serde_json::json!({
                    "call_id": call_id,
                    "interrupted": reason,
                    "elapsed_seconds": elapsed_seconds,
                    "error": detail,
                });
                if let (Some(data), Some(progress)) = (data.as_object_mut(), progress.as_object()) {
                    data.extend(progress.clone());
                }
                data
            },
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
        .with_scaffold_hash(Some(self.scaffold_hash.clone()))
        .with_agent_sessions(sessions);
        let control_reason = matches!(reason, "paused" | "cancelled");
        let persisted = self.runner.workflow_store.with_run_lock(
            &self.runner.run_id,
            |locked| {
                // Generation-free dispatches still belong to the epoch they
                // started in. Refuse stale control and delivery evidence before
                // touching a post-restart slot or emitting its completion event.
                // Issue 291: and never by a session a resume replaced.
                self.runner.v2_store.require_session_owner()?;
                let current = locked.load_state(&self.runner.run_id)?;
                let control_state = matches!(
                    current.status,
                    archon_workflow::RunStatus::Paused
                        | archon_workflow::RunStatus::Cancelled
                );
                // A control stop bumped the generation; a delivery failure
                // did not, so the dispatching generation must still own it.
                // Round 7 (#285): a terminal host stop happened before any
                // later pause, restart or force-accept, so only executor
                // ownership is required; the generation may have moved.
                let owned = dispatch_generation.is_none_or(|generation| {
                    if reason == TERMINAL_HOST_STOP_REASON {
                        current
                            .executor_generation
                            .is_none_or(|owner| owner <= generation)
                    } else if control_reason {
                        current.generation == generation.saturating_add(1)
                    } else {
                        current.generation == generation
                    }
                });
                if (control_reason && !control_state) || !owned {
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
                // An interruption completes nothing: it drops a stale
                // completed mark but never creates a checkpoint.
                self.forget_completed_call(&record.call.id)?;
                self.emit_call_finished_event(&record);
                Ok(event)
            },
        );
        let event = match persisted {
            Ok(event) => event,
            Err(err) => {
                tracing::warn!(%call_id, reason, %err, "interrupted call evidence not saved");
                return Err(err);
            }
        };
        self.clear_inflight(call_id);
        if reason == TERMINAL_HOST_STOP_REASON {
            self.mark_executed(&record, record.status).await;
        }
        if let Some(event) = event
            && let Err(err) = self.runner.client.ui_sink.emit(event).await
        {
            tracing::warn!(%call_id, reason, %err, "interrupted call UI event not delivered");
        }
        Ok(())
    }
}

/// The progress facts of a call's sessions, for its interrupted record: the
/// most turns any session reached, the last tool call of the last session that
/// made one, every path the write tools touched, and each session's own row.
/// Empty (no keys) when no session reported anything.
pub(super) fn interruption_progress(sessions: &[String]) -> serde_json::Value {
    let rows = sessions
        .iter()
        .flat_map(|session| archon_tools::session_progress::snapshot_for(session))
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return serde_json::json!({});
    }
    let turns = rows.iter().map(|row| row.turns).max().unwrap_or(0);
    let last_tool_call = rows.iter().rev().find_map(|row| row.last_tool_call.clone());
    let touched = rows
        .iter()
        .flat_map(|row| row.touched_paths.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    serde_json::json!({
        "turns": turns,
        "last_tool_call": last_tool_call,
        "touched_paths": touched,
        "agent_progress": rows,
    })
}

#[cfg(test)]
#[path = "workflow_live_v2_script_host_interrupt_tests.rs"]
mod workflow_live_v2_script_host_interrupt_tests;
