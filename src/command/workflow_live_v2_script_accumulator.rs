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
    pub(super) terminal_host_stop: bool,
    /// Issue-285: a JavaScript script drives this host, so a terminal stop is
    /// sticky and refuses later calls. The native lifecycle driver is host code
    /// and keeps its own host-built fallback report.
    pub(super) script_driven: bool,
    /// Consecutive calls that failed without ever starting. Run-scoped: the
    /// bound only means anything across calls.
    pub(super) never_started: NeverStartedStreak,
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
            terminal_host_stop: false,
            script_driven: false,
            never_started: NeverStartedStreak::default(),
        }
    }
}

impl WorkflowScriptAccumulator {
    /// A trusted terminal stop that a script can no longer change.
    pub(super) fn terminal_locked(&self) -> bool {
        self.terminal_host_stop && self.script_driven
    }
}
