//! The pause of an acceptance round and of a run end (Issue 316).
//!
//! Every such pause goes through
//! [`archon_workflow::control_pause::pause_owned`]: the ownership check and
//! the pause in ONE run-lock critical section, as its owner requires. A run
//! end, and a round it re-enters, pause for their executor
//! (`PauseOwner::Executor`): an operator edit that kept the executor (a
//! restart of a stage or item, a force-accept) never turns their pause into
//! a cancellation, and a resume that replaced the executor refuses it. A
//! round the script host dispatched keeps the exact generation it was
//! dispatched at (`PauseOwner::Generation`): the host re-dispatches a call
//! an edit superseded.

use archon_workflow::control_pause::PauseOwner;
use archon_workflow::{WorkflowResult, WorkflowStore};

/// Pauses `run_id` for `owner` with `detail` as the evidence; see
/// [`archon_workflow::control_pause::pause_owned`].
pub(in super::super) fn pause(
    store: &WorkflowStore,
    run_id: &str,
    owner: PauseOwner,
    detail: serde_json::Value,
) -> WorkflowResult<WorkflowResult<u64>> {
    #[cfg(test)]
    if let Some(hook) = BEFORE_PAUSE.with(|hook| hook.borrow_mut().take()) {
        hook();
    }
    archon_workflow::control_pause::pause_owned(store, run_id, owner, detail)
}

#[cfg(test)]
thread_local! {
    /// Runs once just before a pause takes the run lock: where an operator
    /// edit would race it (the Issue 316 review).
    pub(in super::super) static BEFORE_PAUSE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}
