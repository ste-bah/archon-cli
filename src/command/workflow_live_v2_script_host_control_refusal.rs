// WorkflowScriptHost: a run control refusal ends the script's host calls
// (Issue 329).
//
// A host call that run control stops (a pause, a cancel, or a stale-session
// refusal after a resume gave the run to a newer executor) rejects with a
// control error the script can catch. A script that caught it and called
// again was refused again, at once, and could loop without end: each refusal
// is instant, so the CPU watchdog, which pauses across host calls, never
// fired. A pause request in such a loop joined the pause each time, with a
// new event and record.
//
// The first control refusal is now sticky for the session. Once it has
// reached the script, every call the script issues after that is refused
// with it, before it reads or writes anything. Calls the script issued with
// it -- siblings in the same pool, already on their way -- still run, so a
// sibling's pause still joins the pause in force. The first refused call
// starts the script's one post-stop budget
// (`WorkflowScriptAccumulator::session_stopped`, Issue 285): a script that
// keeps calling is interrupted and the executor exits, while a script that
// unwinds and awaits its pending calls is not cut short. The refusal keeps
// its kind: a later call of a paused session is refused as a pause, of a
// stale one as a stale session. Host-driven callers (the native lifecycle
// driver, a superseded call run again) issue no script call and are not
// gated.

use super::*;

use super::workflow_live_v2_script_accumulator::ControlRefusal;

impl WorkflowScriptHost {
    /// Runs one host-driven call (no script issue order).
    pub(crate) async fn execute(
        &self,
        method: String,
        payload: String,
    ) -> archon_workflow::WorkflowResult<String> {
        self.execute_issued(method, payload, None).await
    }

    /// Runs one host call the script issued as its `issued`-th, unless the
    /// script issued it after a run control refusal reached it: then the
    /// call is refused with that refusal at once.
    pub(in super::super) async fn execute_issued(
        &self,
        method: String,
        payload: String,
        issued: Option<u64>,
    ) -> archon_workflow::WorkflowResult<String> {
        let refused = {
            let mut acc = self.accumulator.lock().await;
            match acc.control_refusal.as_mut() {
                Some(refusal)
                    if refusal
                        .delivered_at
                        .zip(issued)
                        .is_some_and(|(delivered, issued)| issued > delivered) =>
                {
                    refusal.called_again = true;
                    Some(refusal.clone())
                }
                _ => None,
            }
        };
        if let Some(refusal) = refused {
            return Err(self.repeated_refusal(&method, refusal));
        }
        let result = self.execute_host_call(method, payload).await;
        let refusal = match &result {
            Err(WorkflowError::ControlPaused(message)) => Some((true, message)),
            Err(WorkflowError::ControlCancelled(message)) => Some((false, message)),
            _ => None,
        };
        if let Some((paused, message)) = refusal {
            let mut acc = self.accumulator.lock().await;
            if acc.control_refusal.is_none() {
                tracing::warn!(
                    run_id = %self.runner.run_id,
                    paused,
                    %message,
                    "run control refused a host call; the script's later calls are refused"
                );
                acc.control_refusal = Some(ControlRefusal {
                    paused,
                    message: message.clone(),
                    delivered_at: None,
                    called_again: false,
                });
            }
        }
        result
    }

    /// Records that a control refusal (`result`) is on its way to the
    /// script, which has issued `issued` calls so far.
    pub(in super::super) async fn note_delivered(
        &self,
        result: &archon_workflow::WorkflowResult<String>,
        issued: &std::sync::atomic::AtomicU64,
    ) {
        if !matches!(
            result,
            Err(WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_))
        ) {
            return;
        }
        let issued = issued.load(std::sync::atomic::Ordering::SeqCst);
        if let Some(refusal) = self.accumulator.lock().await.control_refusal.as_mut() {
            refusal.delivered_at.get_or_insert(issued);
        }
    }

    fn repeated_refusal(&self, method: &str, refusal: ControlRefusal) -> WorkflowError {
        // Tests act here: at the first call the script makes past a refusal.
        #[cfg(test)]
        super::super::super::workflow_live_v2_run::terminal_test_support::unwind(
            self.runner.workflow_store.run_dir(&self.runner.run_id),
        );
        let message = format!(
            "{}; host call '{method}' refused: the script of run {} already received that run control refusal, and this session makes no further host call",
            refusal.message, self.runner.run_id
        );
        if refusal.paused {
            WorkflowError::ControlPaused(message)
        } else {
            WorkflowError::ControlCancelled(message)
        }
    }
}
