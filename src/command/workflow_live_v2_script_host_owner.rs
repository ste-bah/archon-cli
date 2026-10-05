// WorkflowScriptHost: executor ownership of every dispatch and write
// (Issue 291).
//
// A resume hands the run to a newer executor while an older one may still
// run its script: a script that caught the control error keeps calling. That
// stale session must dispatch nothing, record nothing and change no run
// state. Its result store is bound to the generation its executor started
// under (`bind_session_executor`), and the shared rule
// (`control_pause::require_executor`, applied by the store) is checked:
// - before a call does anything, and whenever a generation is sampled for a
//   dispatch (`owned_generation`), so a generation read after a takeover is
//   never taken for this executor's own;
// - under the run lock, at every write (`require_session_owner`).
// Every refusal is logged by the store and returned as a control
// cancellation naming the stale session: nothing is dropped silently.

use super::*;

impl WorkflowScriptHost {
    /// The run's generation now, read only while this executor owns the run.
    pub(in super::super) fn owned_generation(&self) -> archon_workflow::WorkflowResult<u64> {
        let run = self.runner.workflow_store.load_state(&self.runner.run_id)?;
        self.runner.v2_store.require_session_executor(&run)?;
        Ok(run.generation)
    }

    /// `operation` under the run lock, after the session-owner check.
    pub(super) fn with_owned_run_lock<T>(
        &self,
        operation: impl FnOnce(&archon_workflow::WorkflowStore) -> archon_workflow::WorkflowResult<T>,
    ) -> archon_workflow::WorkflowResult<T> {
        self.runner
            .workflow_store
            .with_run_lock(&self.runner.run_id, |locked| {
                self.runner.v2_store.require_session_owner()?;
                operation(locked)
            })
    }

    /// Marks the call's stage running, only while this executor owns the run.
    pub(super) fn mark_call_running_owned(
        &self,
        call_id: &str,
    ) -> archon_workflow::WorkflowResult<()> {
        self.with_owned_run_lock(|locked| {
            mark_v2_call_running(locked, &self.runner.run_id, call_id)
        })
    }

    /// Whether this session may still append to the run's event log: not
    /// once a newer executor owns the run. A refusal is logged by the store;
    /// an unreadable state proves nothing and the event is emitted
    /// best-effort as before. Unlocked: events are emitted under the run
    /// lock by some callers.
    pub(super) fn may_emit_events(&self) -> bool {
        !matches!(
            self.owned_generation(),
            Err(WorkflowError::ControlCancelled(_))
        )
    }
}
