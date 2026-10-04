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

    pub(super) async fn interrupt_terminal_calls(&self) -> archon_workflow::WorkflowResult<()> {
        let pending = self
            .runner
            .pending_calls
            .lock()
            .map_err(|_| {
                WorkflowError::StateCorrupt("pending workflow calls lock poisoned".into())
            })?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        archon_tools::bash::end_process_groups_of(&self.runner.run_id);
        let error = WorkflowError::TerminalHostCall("host terminal stop ended pending work".into());
        for call in pending {
            self.save_interrupted_call_record(
                &call.execution,
                "terminal_host_stop",
                &error,
                call.started.elapsed(),
                call.attempt,
                &call.input_hash,
                call.source_fingerprint,
                call.generation,
            )
            .await?;
        }
        Ok(())
    }
}
