// One `__archonHost` call: the bridge from a workflow.js host call into the
// script host.
//
// It lives beside `workflow_live_v2_script.rs` to hold the 500-line ceiling.
// Each call takes its issue order when the script makes it (Issue 329), runs
// with the CPU watchdog paused, and records what the HOST raised -- a
// notification failure, a run control stop, an infrastructure fault (Issue
// 324) -- so the run outcome never depends on what the script did with the
// rejection.

use super::*;

/// What every host call of one script shares.
#[derive(Clone)]
pub(super) struct ScriptHostCallBridge {
    pub(super) host: Arc<WorkflowScriptHost>,
    pub(super) watchdog: WorkflowJsWatchdog,
    /// Issue 364: the script thread's heartbeat; a call's start and end are
    /// progress.
    pub(super) heartbeat:
        archon_workflow::v2::script::script_thread_heartbeat::ScriptThreadHeartbeat,
    /// A notification failure the HOST raised.
    pub(super) notification: Arc<StdMutex<Option<String>>>,
    /// The run control stop a call resolved to.
    pub(super) control: Arc<StdMutex<Option<HostControlStop>>>,
    /// Issue 329: the order in which the script issued its host calls.
    pub(super) issued: Arc<std::sync::atomic::AtomicU64>,
    /// Issue 324: the host fault that stopped a script that stops on one.
    pub(super) fault: Arc<StdMutex<Option<WorkflowError>>>,
}

impl ScriptHostCallBridge {
    /// Runs one host call. The issue order is taken now, when the script
    /// makes the call, not when the returned future first runs.
    pub(super) fn call(
        self,
        method: String,
        payload: String,
    ) -> impl std::future::Future<Output = rquickjs::Result<String>> {
        let order = self
            .issued
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        async move {
            let Self {
                host,
                watchdog,
                heartbeat,
                notification,
                control,
                issued,
                fault,
            } = self;
            watchdog.pause();
            heartbeat.beat();
            let result = Box::pin(host.execute_issued(method, payload, Some(order))).await;
            host.note_delivered(&result, &issued).await;
            // Issue-285/329: refused calls are instant, so after
            // a terminal or control stop one budget runs on.
            if host.accumulator.lock().await.session_stopped() {
                watchdog.start_terminal_budget();
            }
            // Issue 364: a call that finished on its first poll still gives
            // the thread's timers and sockets a turn (defense in depth).
            archon_workflow::v2::script::host_call_yield::yield_to_script_runtime().await;
            heartbeat.beat();
            watchdog.resume();
            if let Err(WorkflowError::NotificationDelivery(message)) = &result
                && let Ok(mut slot) = notification.lock()
            {
                slot.get_or_insert_with(|| message.clone());
            }
            if let Err(err) = &result
                && host.runner.stops_on_host_fault
                && let Some(copy) = copy_host_infrastructure_fault(err)
                && let Ok(mut slot) = fault.lock()
            {
                slot.get_or_insert(copy);
            }
            result.or_else(|err| {
                // Issue-253: run control resolves to a typed
                // envelope; every other error rejects as before.
                control_envelope(&host.runner.workflow_store, &host.runner.run_id, &err)
                    .map(|(envelope, observed)| {
                        if let Ok(mut slot) = control.lock() {
                            slot.get_or_insert(observed);
                        }
                        envelope
                    })
                    .ok_or_else(|| {
                        rquickjs::Error::new_from_js_message(
                            "archon workflow host",
                            "string",
                            err.to_string(),
                        )
                    })
            })
        }
    }
}
