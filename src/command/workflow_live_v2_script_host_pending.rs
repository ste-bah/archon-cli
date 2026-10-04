//! Live-call ownership is kept in memory as well as in crash-recovery markers.
//! A terminal watchdog is a host interruption, not a process crash: journal
//! pending work before dropping QuickJS futures, including post-apply audits.
use super::*;
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Clone)]
pub(super) struct PendingCall {
    execution: WorkflowV2CallExecution,
    attempt: u32,
    input_hash: String,
    source_fingerprint: Option<String>,
    generation: Option<u64>,
    started: Instant,
}

pub(super) type PendingCalls = Arc<StdMutex<BTreeMap<String, PendingCall>>>;

impl WorkflowScriptHost {
    pub(super) fn track_pending_call(
        &self,
        execution: &WorkflowV2CallExecution,
        attempt: u32,
        input_hash: &str,
        source_fingerprint: Option<String>,
        generation: Option<u64>,
        started: Instant,
    ) -> archon_workflow::WorkflowResult<()> {
        self.runner
            .pending_calls
            .lock()
            .map_err(|_| {
                WorkflowError::StateCorrupt("pending workflow calls lock poisoned".into())
            })?
            .insert(
                execution.call.id.clone(),
                PendingCall {
                    execution: execution.clone(),
                    attempt,
                    input_hash: input_hash.to_string(),
                    source_fingerprint,
                    generation,
                    started,
                },
            );
        Ok(())
    }

    pub(super) fn forget_pending_call(&self, id: &str) {
        if let Ok(mut calls) = self.runner.pending_calls.lock() {
            calls.remove(id);
        }
    }

    /// A started record can survive a projection or UI error. Keep its
    /// ownership until terminal cleanup closes it; an unreadable record also
    /// retains ownership so a storage error cannot silently orphan work.
    pub(super) fn forget_unwritten_pending_call(&self, id: &str) {
        if self
            .runner
            .v2_store
            .load_call_record(id)
            .is_ok_and(|record| {
                record.is_none_or(|record| record.status != WorkflowV2Status::Running)
            })
        {
            self.forget_pending_call(id);
        }
    }

    /// Round 7 (#285): never fails. The terminal stop happened first, so a
    /// later operator edit cannot turn it into a control outcome; a record
    /// that cannot be saved is logged as evidence and the stop still ends
    /// the run.
    pub(super) async fn interrupt_terminal_calls(&self) {
        #[cfg(test)]
        super::super::workflow_live_v2_run::terminal_test_support::unwind(
            self.runner.workflow_store.run_dir(&self.runner.run_id),
        );
        let pending = match self.runner.pending_calls.lock() {
            Ok(calls) => calls.values().cloned().collect::<Vec<_>>(),
            Err(poisoned) => poisoned.into_inner().values().cloned().collect(),
        };
        archon_tools::bash::end_process_groups_of(&self.runner.run_id);
        let error = WorkflowError::TerminalHostCall("host terminal stop ended pending work".into());
        for call in pending {
            let id = call.execution.call.id.clone();
            if let Err(err) = self
                .save_interrupted_call_record(
                    &call.execution,
                    "terminal_host_stop",
                    &error,
                    call.started.elapsed(),
                    call.attempt,
                    &call.input_hash,
                    call.source_fingerprint,
                    call.generation,
                )
                .await
            {
                tracing::warn!(call_id = %id, %err, "terminal stop interruption record not saved");
                self.forget_pending_call(&id);
            }
        }
    }
}
