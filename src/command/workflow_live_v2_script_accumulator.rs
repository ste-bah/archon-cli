//! The run-scoped accumulator a `WorkflowScriptHost` folds every call outcome
//! into. Split out of `workflow_live_v2_script.rs` to hold the 500-line ceiling.

use super::*;

pub(super) struct WorkflowScriptAccumulator {
    pub(super) status: WorkflowV2Status,
    pub(super) completed: usize,
    pub(super) executed: usize,
    pub(super) reused: usize,
    pub(super) calls: Vec<WorkflowV2HostCall>,
    pub(super) failed_call: Option<String>,
    pub(super) failed_result_path: Option<String>,
    pub(super) next_action: Option<String>,
    /// Issue 332: the error text of a failed workflow.js.
    pub(super) script_error: Option<String>,
    pub(super) terminal_host_stop: bool,
    /// Issue-285: a JavaScript script drives this host, so a terminal stop is
    /// sticky and refuses later calls. The native lifecycle driver is host code
    /// and keeps its own host-built fallback report.
    pub(super) script_driven: bool,
    /// Consecutive calls that failed without ever starting. Run-scoped: the
    /// bound only means anything across calls.
    pub(super) never_started: NeverStartedStreak,
    /// Issue 329: the first run control refusal (pause, cancel, or a stale
    /// session) a host call of this session received. Sticky: every later
    /// call is refused with it (`workflow_live_v2_script_host_control_refusal.rs`).
    pub(super) control_refusal: Option<ControlRefusal>,
}

/// A run control refusal, kept to refuse every later call of the session.
#[derive(Clone, Debug)]
pub(super) struct ControlRefusal {
    pub(super) paused: bool,
    pub(super) message: String,
    /// How many calls the script had issued when the refusal reached it.
    /// A call issued later is the script calling past the refusal; one
    /// issued with it (a sibling in the same pool) still runs.
    pub(super) delivered_at: Option<u64>,
    /// The script called again after the refusal: it will not unwind alone.
    pub(super) called_again: bool,
}

impl ControlRefusal {
    /// The refusal as its control error words it, for a script stopped
    /// because it kept calling after it.
    pub(super) fn stopped_script_text(&self) -> String {
        let error = if self.paused {
            WorkflowError::ControlPaused(self.message.clone())
        } else {
            WorkflowError::ControlCancelled(self.message.clone())
        };
        format!("{error}; the script kept calling the host after this refusal and was stopped")
    }
}

impl Default for WorkflowScriptAccumulator {
    fn default() -> Self {
        Self {
            status: WorkflowV2Status::Accepted,
            completed: 0,
            executed: 0,
            reused: 0,
            calls: Vec::new(),
            failed_call: None,
            failed_result_path: None,
            next_action: None,
            script_error: None,
            terminal_host_stop: false,
            script_driven: false,
            never_started: NeverStartedStreak::default(),
            control_refusal: None,
        }
    }
}

impl WorkflowScriptAccumulator {
    /// A trusted terminal stop that a script can no longer change.
    pub(super) fn terminal_locked(&self) -> bool {
        self.terminal_host_stop && self.script_driven
    }

    /// Whether the script's one post-stop budget runs: after a terminal host
    /// stop, or once the script calls again after a run control refusal. A
    /// script that unwinds from the refusal and awaits its pending calls is
    /// not cut short.
    pub(super) fn session_stopped(&self) -> bool {
        self.terminal_host_stop
            || self
                .control_refusal
                .as_ref()
                .is_some_and(|refusal| refusal.called_again)
    }
}

impl WorkflowScriptHost {
    pub(super) async fn mark_terminal(
        &self,
        record: &WorkflowV2CallRecord,
        result_path: String,
        next_action: String,
    ) {
        let mut acc = self.accumulator.lock().await;
        if acc.terminal_locked() {
            return;
        }
        // Issue 337: persisted like a deliberate stop (one mechanism).
        self.persist_call_terminal_stop(record);
        acc.terminal_host_stop = true;
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
        drop(acc);
        #[cfg(test)]
        super::super::workflow_live_v2_run::terminal_test_support::stop(
            self.runner.workflow_store.run_dir(&self.runner.run_id),
        );
    }
}
