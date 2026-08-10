//! The IDE side of the agent's permission gate (issue #26, item 5).
//!
//! [`Agent`](archon_core::agent::Agent) asks for permission by emitting
//! `PermissionRequired` and then blocking on `permission_response_rx`. When
//! that receiver is `None` it auto-approves instead
//! (`archon-core/src/agent/permission_gate.rs`), which is why the first IDE
//! slice shipped with no tools at all. [`PermissionBridge`] is the missing
//! half: it owns the sending end of that channel, mints a correlation id for
//! each request so a decision cannot be applied to the wrong tool, and refuses
//! anything it did not ask for.
//!
//! Correlation is not decoration. The agent's channel carries a bare `bool`
//! with no request identity, so a duplicated or late click would otherwise sit
//! buffered and silently approve whatever the agent asked next.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::sync::mpsc;

/// Sender half of the agent's permission channel, plus the bookkeeping that
/// makes a `bool` channel safe to expose over JSON-RPC.
pub struct PermissionBridge {
    responses: mpsc::Sender<bool>,
    /// The request the agent is currently blocked on, if any.
    pending: Mutex<Option<String>>,
    next_id: AtomicU64,
    /// Whether the connected client advertised an approval UI.
    client_can_answer: AtomicBool,
}

impl PermissionBridge {
    /// Wrap the sender paired with the receiver installed on the agent.
    ///
    /// Starts out assuming the client cannot answer. A client that has not
    /// completed `archon/initialize` has advertised nothing, and the safe
    /// reading of "nothing" is that no one is there to approve.
    pub fn new(responses: mpsc::Sender<bool>) -> Self {
        Self {
            responses,
            pending: Mutex::new(None),
            next_id: AtomicU64::new(1),
            client_can_answer: AtomicBool::new(false),
        }
    }

    /// Record whether the connected client advertised `toolExecution`, i.e.
    /// whether it has an allow/deny UI at all.
    pub fn set_client_can_answer(&self, can_answer: bool) {
        self.client_can_answer.store(can_answer, Ordering::Relaxed);
    }

    /// Whether the connected client can answer a permission prompt.
    pub fn client_can_answer(&self) -> bool {
        self.client_can_answer.load(Ordering::Relaxed)
    }

    /// Refuse the outstanding request on behalf of a client that cannot
    /// answer it.
    ///
    /// The agent's own fallback is a 120-second timeout that then denies, so
    /// the outcome is the same either way; this just stops a client with no
    /// approval UI from freezing the session for two minutes per tool call.
    pub fn deny_unanswerable(&self) {
        let Ok(mut pending) = self.pending.lock() else {
            return;
        };
        if pending.is_none() {
            return;
        }
        if let Err(error) = self.responses.try_send(false) {
            tracing::warn!(%error, "could not auto-deny a permission request");
            return;
        }
        *pending = None;
    }

    /// Record that the agent is now waiting, and return the id the IDE must
    /// echo back.
    ///
    /// A request that arrives while another is outstanding replaces it. The
    /// agent is single-threaded through `preflight_tools`, so that can only
    /// mean the previous request already ended (denied on timeout, or the turn
    /// was cancelled) — and the stale id must stop being answerable.
    pub fn open_request(&self) -> String {
        let id = format!("perm-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        if let Ok(mut pending) = self.pending.lock()
            && let Some(previous) = pending.replace(id.clone())
        {
            tracing::debug!(%previous, %id, "IDE permission request superseded");
        }
        id
    }

    /// Stop accepting answers for the outstanding request.
    ///
    /// Called when the agent announces it has stopped waiting. Without this a
    /// request that timed out would still be answerable, and the `true` would
    /// sit in the channel until the *next* tool picked it up.
    pub fn close_request(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            *pending = None;
        }
    }

    /// Deliver the user's decision for `request_id`.
    ///
    /// Returns `Err` with a human-readable reason rather than silently
    /// succeeding: an answer to a request nobody is waiting on is a client
    /// bug, and swallowing it would hide the fact that the decision had no
    /// effect.
    pub fn respond(&self, request_id: &str, approved: bool) -> Result<(), String> {
        // Checked, cleared and sent under one lock so two clicks cannot both
        // pass the check and enqueue two answers for one question.
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "permission bridge is poisoned".to_string())?;
        match pending.as_deref() {
            None => Err("no permission request is awaiting a decision".to_string()),
            Some(open) if open != request_id => Err(format!(
                "permission request {request_id} is not the one awaiting a decision ({open})"
            )),
            Some(_) => {
                self.responses.try_send(approved).map_err(|error| {
                    format!("the agent is not reading permission answers: {error}")
                })?;
                *pending = None;
                Ok(())
            }
        }
    }

    /// Whether a decision is currently outstanding. Test/diagnostic only.
    pub fn is_waiting(&self) -> bool {
        self.pending.lock().map(|p| p.is_some()).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge() -> (PermissionBridge, mpsc::Receiver<bool>) {
        let (tx, rx) = mpsc::channel(1);
        (PermissionBridge::new(tx), rx)
    }

    #[tokio::test]
    async fn an_answer_reaches_the_agent_channel() {
        let (bridge, mut rx) = bridge();
        let id = bridge.open_request();

        bridge.respond(&id, true).expect("answer accepted");

        assert_eq!(rx.recv().await, Some(true));
        assert!(!bridge.is_waiting());
    }

    #[test]
    fn an_unsolicited_answer_is_refused() {
        let (bridge, _rx) = bridge();

        let error = bridge.respond("perm-1", true).expect_err("must refuse");

        assert!(error.contains("no permission request"), "{error}");
    }

    #[test]
    fn a_stale_id_cannot_answer_the_current_request() {
        let (bridge, _rx) = bridge();
        let first = bridge.open_request();
        bridge.close_request();
        let second = bridge.open_request();

        let error = bridge.respond(&first, true).expect_err("must refuse");

        assert!(error.contains(&second), "{error}");
    }

    /// A client with no approval UI must not be able to stall the agent, and
    /// must never be treated as consent.
    #[tokio::test]
    async fn a_client_without_an_approval_ui_gets_an_immediate_refusal() {
        let (bridge, mut rx) = bridge();
        assert!(!bridge.client_can_answer(), "default must be fail-closed");
        bridge.open_request();

        bridge.deny_unanswerable();

        assert_eq!(rx.recv().await, Some(false));
        assert!(!bridge.is_waiting());
    }

    #[test]
    fn a_second_click_on_the_same_request_is_refused() {
        let (bridge, _rx) = bridge();
        let id = bridge.open_request();
        bridge.respond(&id, false).expect("first answer accepted");

        let error = bridge.respond(&id, true).expect_err("must refuse");

        assert!(error.contains("no permission request"), "{error}");
    }
}
