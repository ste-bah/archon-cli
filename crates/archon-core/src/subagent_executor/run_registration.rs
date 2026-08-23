//! The `Running` registration a subagent run holds, released even if the run
//! is never finished.
//!
//! # The failure this exists to stop
//!
//! Registration and release used to be two statements in `run.rs`: register,
//! await the run, then record a terminal status. That is correct for every way
//! a run can *finish* and wrong for the one way it can stop without finishing —
//! the future being dropped. A cancelled branch, an outer timeout, an aborted
//! task: execution stops at an `.await` inside the run and the release
//! statement below it is simply never reached, so the entry stays `Running` for
//! the life of the process.
//!
//! `register_with_id` rejects a duplicate id only while the existing entry is
//! `Running`, and a workflow branch re-dispatches under the SAME id. So the
//! stranded entry did not merely leak: it made the retry impossible. Observed
//! live, as the error that ended a fifteen-task run at its third task —
//! `subagent already exists and is running:
//! wf-…-implement-tdl-020-11-0-attempt-1-…-coder` — reported to the write layer
//! as an unclassified branch failure, which reads as the agent's fault.
//!
//! Holding the registration in a guard makes the release unconditional: a drop
//! is a path the compiler already runs, so there is no path left that returns
//! without taking it. `BuildCacheLease` releases its slot on drop for the same
//! reason and against the same failure.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::subagent::SubagentManager;

/// A held `Running` registration.
pub(super) struct RunRegistration {
    manager: Arc<Mutex<SubagentManager>>,
    id: String,
    /// Which occupancy of `id` this guard speaks for.
    ///
    /// An id is reused: a retry re-registers under the same one, which is the
    /// whole reason a stranded entry was fatal. So the release has to be scoped
    /// to the run that took it, or a late release would fail its own successor.
    generation: u64,
    /// Whether the ordinary completion path has already recorded a terminal
    /// status. Set last, so a completion interrupted part-way through is still
    /// covered by the drop.
    settled: bool,
}

impl RunRegistration {
    /// Take the guard for the registration `id` currently holds.
    ///
    /// The generation is read here rather than passed in so a caller cannot
    /// hold a guard for a run that no longer exists; `None` from the manager
    /// means the entry was already gone, and the guard then has nothing to
    /// release.
    pub(super) async fn take(manager: Arc<Mutex<SubagentManager>>, id: String) -> Self {
        let generation = manager.lock().await.generation(&id).unwrap_or_default();
        Self {
            manager,
            id,
            generation,
            settled: false,
        }
    }

    /// The run reported a terminal status through the normal path; the drop
    /// below has nothing left to do.
    pub(super) fn settle(&mut self) {
        self.settled = true;
    }
}

impl Drop for RunRegistration {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        // Named as what it is. A run that vanished mid-flight has no result and
        // no error of its own, and calling it "failed" without saying why would
        // send the next reader looking for a fault in the agent.
        let reason = format!(
            "subagent run '{}' stopped without reporting a result — it was cancelled, timed out, \
             or its task was dropped",
            self.id
        );
        // Uncontended is the ordinary case: the run that held this guard is
        // gone, so nothing is mid-call on the manager. Taking it inline keeps
        // the release synchronous with the drop, which is what makes an
        // immediately following retry see a free id.
        if let Ok(mut manager) = self.manager.try_lock() {
            let _ = manager.mark_failed_at_generation(&self.id, self.generation, reason);
            manager.cleanup_agent(&self.id);
            return;
        }
        // Contended: `Drop` cannot await — Rust has no async destructor — so the
        // release is handed to the runtime, which is the standard shape for
        // this. It costs synchrony: until the spawned release is served the id
        // is still `Running`, so a retry issued in that window is refused as if
        // nothing had been fixed. That is survivable here because a branch
        // retries after a transport backoff, not immediately, and the window is
        // one lock hold long. It is not survivable silently, which is why the
        // no-runtime case below is an error rather than a debug line.
        let manager = Arc::clone(&self.manager);
        let id = self.id.clone();
        let generation = self.generation;
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            // No runtime means no way to take an async lock, and inventing one
            // here would block a thread that is already unwinding. Loud,
            // because the consequence is an id nothing can reuse.
            tracing::error!(
                subagent_id = %self.id,
                "subagent registration could not be released on drop: manager busy and no runtime"
            );
            return;
        };
        handle.spawn(async move {
            let mut manager = manager.lock().await;
            // Generation-scoped: by the time this is served the retry may have
            // taken the id back, and failing it would report the replacement as
            // broken for its predecessor's reason.
            let _ = manager.mark_failed_at_generation(&id, generation, reason);
            manager.cleanup_agent(&id);
        });
    }
}

#[cfg(test)]
#[path = "run_registration_tests.rs"]
mod tests;
