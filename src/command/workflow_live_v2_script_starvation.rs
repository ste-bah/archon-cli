//! Issue 364: a starved workflow.js thread pauses the run with its evidence.
//!
//! The script thread carries a heartbeat
//! (`archon_workflow::v2::script::script_thread_heartbeat`): a ticker on the
//! script runtime and every host call's start and end beat it. The QuickJS
//! interrupt handler, which here checks the CPU watchdog first and the
//! heartbeat after it, cuts JavaScript that ran a whole no-progress window of
//! CPU with no beat, even while the watchdog is paused for an in-flight call.
//! A monitor thread writes the durable record [`STARVATION_RECORD`] (what was
//! in flight) as soon as the heartbeat is stale while the process burns CPU.
//!
//! A cut ends the run as a resumable PAUSE through the script-error pause,
//! for every script (the authoring bootstrap too), never as a failure: the
//! recorded calls answer from their records on resume.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use archon_workflow::v2::script::script_thread_heartbeat::{
    MonitorHandle, ScriptThreadHeartbeat, Starvation, TickerGuard,
};

use super::workflow_live_v2_script_host_pending::in_flight_summary;
use super::workflow_live_v2_script_watchdog::{WORKFLOW_JS_WATCHDOG, WorkflowJsWatchdog};
use super::*;

/// The durable starvation record, relative to the run directory.
pub(super) const STARVATION_RECORD: &str = "v2/script-thread-starved.json";

/// The no-progress window: twice the CPU watchdog's budget, so a script the
/// watchdog allows (a long computation between host calls) is never cut.
pub(super) const STARVATION_WINDOW: Duration =
    Duration::from_nanos(WORKFLOW_JS_WATCHDOG.as_nanos() as u64 * 2);

#[cfg(not(test))]
const HEARTBEAT_TICK: Duration = Duration::from_secs(1);
#[cfg(test)]
const HEARTBEAT_TICK: Duration = Duration::from_millis(25);

/// Test seam: runs whose CPU watchdog stays paused, as it does while a host
/// call is in flight, so only the heartbeat can end their JavaScript.
#[cfg(test)]
pub(super) static WATCHDOG_PAUSED_RUNS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeSet<String>>,
> = std::sync::LazyLock::new(Default::default);

/// The heartbeat of one script run, with its ticker and monitor.
pub(super) struct ScriptStarvationGuard {
    pub(super) heartbeat: ScriptThreadHeartbeat,
    _ticker: TickerGuard,
    _monitor: MonitorHandle,
}

impl ScriptStarvationGuard {
    /// Starts the heartbeat on the calling (script) thread and installs the
    /// combined interrupt handler of `runtime`.
    pub(super) async fn install(
        host: &Arc<WorkflowScriptHost>,
        runtime: &AsyncRuntime,
        watchdog: &WorkflowJsWatchdog,
    ) -> Self {
        let pending = host.runner.pending_calls.clone();
        let (store, run_id) = (
            host.runner.workflow_store.clone(),
            host.runner.run_id.clone(),
        );
        let heartbeat = ScriptThreadHeartbeat::new(
            STARVATION_WINDOW,
            Box::new(move || in_flight_summary(&pending)),
            Box::new(move |starvation| write_record(&store, &run_id, starvation)),
        );
        #[cfg(test)]
        let watchdog_paused = WATCHDOG_PAUSED_RUNS
            .lock()
            .is_ok_and(|runs| runs.contains(&host.runner.run_id));
        #[cfg(not(test))]
        let watchdog_paused = false;
        let (for_interrupt, watchdog) = (heartbeat.clone(), watchdog.clone());
        runtime
            .set_interrupt_handler(Some(Box::new(move || {
                if !watchdog_paused && watchdog.should_interrupt() {
                    if !watchdog.after_terminal_stop() {
                        for_interrupt.note_watchdog_cut();
                    }
                    return true;
                }
                for_interrupt.should_cut()
            })))
            .await;
        Self {
            _ticker: heartbeat.spawn_ticker(HEARTBEAT_TICK),
            _monitor: heartbeat.spawn_monitor(HEARTBEAT_TICK.max(STARVATION_WINDOW / 16)),
            heartbeat,
        }
    }

    /// The pause a cut ends the run with; `None` when nothing was cut.
    pub(super) async fn finish(
        &self,
        host: &WorkflowScriptHost,
    ) -> Option<archon_workflow::WorkflowResult<WorkflowV2ScriptSummary>> {
        let starvation = self.heartbeat.cut()?;
        self.heartbeat.record(&starvation);
        let in_flight = starvation
            .in_flight
            .iter()
            .filter_map(|call| call["id"].as_str())
            .collect::<Vec<_>>();
        // Stable text (no timings), so a recurrence at the same point is
        // counted as one by the script-error pause.
        let error = format!(
            "workflow.js script thread starved: its JavaScript ran for a whole {STARVATION_WINDOW:?} no-progress window (no heartbeat, no host-call completion) and was interrupted ({} check); in flight: [{}]; evidence: {STARVATION_RECORD}",
            starvation.detected_by,
            in_flight.join(", "),
        );
        tracing::warn!(run_id = %host.runner.run_id, "{error}");
        Some(Err(
            match host
                .pause_on_script_stop(&error, "script_thread_starved")
                .await
            {
                Some(stop) => stop,
                None => WorkflowError::SpecInvalid(format!(
                    "{error}; the host could not persist its pause"
                )),
            },
        ))
    }
}

fn write_record(store: &WorkflowStore, run_id: &str, starvation: &Starvation) {
    tracing::warn!(
        run_id,
        detected_by = %starvation.detected_by,
        state = %starvation.state,
        no_progress_ms = starvation.no_progress_ms,
        in_flight = starvation.in_flight.len(),
        "workflow.js script thread starved"
    );
    if let Err(error) = store.write_run_json(run_id, STARVATION_RECORD, starvation) {
        tracing::warn!(%error, run_id, "script thread starvation record not written");
    }
}

/// Waits for the script's promise, unless the post-stop budget runs out
/// (Issue-285: a script idle forever after a terminal stop runs no JavaScript
/// to interrupt, so the budget also ends the wait) or the heartbeat cut the
/// script (Issue 364); `Err` is the stop's text.
pub(super) async fn settle_or_stop<T>(
    settled: impl Future<Output = T>,
    watchdog: &WorkflowJsWatchdog,
    heartbeat: &ScriptThreadHeartbeat,
) -> Result<T, String> {
    tokio::select! {
        biased;
        settled = settled => Ok(settled),
        () = watchdog.terminal_budget_spent() => Err(format!(
            "workflow.js did not settle within {WORKFLOW_JS_WATCHDOG:?} of the host's terminal stop"
        )),
        () = heartbeat.cut_signal() => Err(
            "workflow.js script thread starved; the host interrupted it".to_string()
        ),
    }
}
