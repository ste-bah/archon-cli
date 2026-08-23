//! Stop an in-flight call when the operator stops the run.
//!
//! # Why checkpoints were not enough
//!
//! Control is observed by [`poll_v2_run_control`], which every branch calls on
//! entry and again after its agent returns. That makes a cancel reliable and
//! makes it SLOW: the only place it can be noticed is between calls, so a run
//! cancelled while four branches are mid-dispatch keeps all four running to
//! completion first. An operator who cancels a stuck run watches it spend
//! whatever remains of four agent calls — potentially hours, and the tokens
//! with them — before anything unwinds. "Cancelled" meant "will stop
//! eventually", and nothing said so.
//!
//! So the call is raced against a watcher. When the watcher sees the run
//! stopped, the work future is DROPPED, which is what actually ends it: the
//! branch jobs are futures joined on one task rather than detached tasks, so
//! dropping propagates the whole way down, and the resources a dropped run
//! holds release themselves (see `run_registration` in `archon-core`).
//!
//! # The watcher only reads
//!
//! [`poll_v2_run_control`] writes: it persists state and produces the canonical
//! typed error. Polling THAT on a timer would turn one state write per call
//! into one every few seconds per branch. So the watcher reads the run's
//! status, and once it sees a stop it defers to `poll_v2_run_control` for the
//! error and the write — one code path producing stops, exactly as before.

use std::future::Future;

use crate::control::poll_v2_run_control;
use crate::store::WorkflowStore;
use crate::{RunStatus, WorkflowError, WorkflowResult};

/// How often the watcher looks.
///
/// Chosen against what it costs and what it saves: a state read is one small
/// JSON file, and the thing being shortened is measured in minutes. Tighter
/// would buy nothing an operator could perceive; looser would leave the
/// behaviour this module exists to remove.
const CONTROL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Run `work`, abandoning it if the run is paused or cancelled meanwhile.
///
/// A read error is NOT a stop. The run's state file is written by other
/// processes and a torn or briefly-locked read says nothing about the
/// operator's intent; treating it as a cancel would kill live work over a
/// transient. The watcher simply looks again, and the checkpoint on either side
/// of this call still reports anything genuinely wrong with the store.
///
/// # What dropping `work` costs
///
/// Cancelling a future in Rust means dropping it, and whether that is safe is a
/// question about the state it was holding — one that has to be asked of the
/// whole chain beneath it, not just of this function. Here that chain ends at a
/// dispatched agent call, whose one piece of state outliving the future was its
/// subagent registration; that now releases on drop (`run_registration` in
/// `archon-core`), which is what makes abandoning the call safe rather than
/// merely quick. A branch's edits live in its own worktree and are applied from
/// a manifest only on success, so an abandoned call contributes nothing to
/// apply.
pub async fn until_run_stops<T>(
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
    work: impl Future<Output = WorkflowResult<T>>,
) -> WorkflowResult<T> {
    tokio::pin!(work);
    // An `Interval` rather than a fresh `sleep` per iteration. A `sleep` built
    // inside the loop is a new timer each time round, which is the documented
    // way to get a watcher that does not tick on the schedule it appears to;
    // one interval, ticked, is the tool for a repeating check. Its first tick
    // is immediate, which is wanted: a run cancelled BEFORE the call started is
    // then noticed at once rather than one whole period later.
    let mut watch = tokio::time::interval(CONTROL_POLL_INTERVAL);
    loop {
        tokio::select! {
            // Biased so a finished call is taken as finished. Without it a call
            // that completed in the same tick the run was cancelled could be
            // discarded at random, which would make a cancel destroy work that
            // had already succeeded.
            biased;
            result = &mut work => return result,
            _ = watch.tick() => {
                if !run_has_stopped(store, run_id) {
                    continue;
                }
                // `work` is dropped here, with this function's frame.
                return Err(stop_error(store, run_id, call_id));
            }
        }
    }
}

/// Whether the operator has stopped this run. Read-only.
fn run_has_stopped(store: &WorkflowStore, run_id: &str) -> bool {
    store
        .load_state(run_id)
        .is_ok_and(|run| matches!(run.status, RunStatus::Paused | RunStatus::Cancelled))
}

/// The typed error for a stop, from the one function that produces them.
///
/// If the checkpoint disagrees — the run resumed between the watcher's read and
/// this call — the work has already been abandoned and cannot be recovered, so
/// this reports a cancellation rather than claiming success it does not have.
fn stop_error(store: &WorkflowStore, run_id: &str, call_id: &str) -> WorkflowError {
    match poll_v2_run_control(store, run_id, call_id) {
        Err(err) => err,
        Ok(()) => WorkflowError::ControlCancelled(format!(
            "run stop observed while V2 call '{call_id}' was in flight; the call was abandoned"
        )),
    }
}

#[cfg(test)]
#[path = "control_race_tests.rs"]
mod tests;
