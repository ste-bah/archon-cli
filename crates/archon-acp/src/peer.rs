//! The far end of the connection, and how to ask it something (#189 Phase 11).
//!
//! Two things live here that the serve loop should not have to think about:
//! writing is serialised through one channel so two tasks cannot interleave
//! halves of a line, and a request this side sends is matched to the reply that
//! comes back later on the same stream the client's own requests arrive on.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{mpsc, oneshot};

use crate::jsonrpc::{Incoming, Notification, Request, Response};
use crate::protocol::{
    PermissionOption, PermissionOptionKind, RequestPermissionRequest, RequestPermissionResponse,
    SessionNotification, SessionUpdate, ToolCallRef,
};

/// The client, as this side can address it.
pub struct Peer {
    outgoing: mpsc::Sender<String>,
    next_id: AtomicU64,
    /// Requests sent and not yet answered.
    ///
    /// A `std::sync::Mutex` rather than tokio's: nothing is awaited while it is
    /// held, and using the async one would invite exactly that.
    pending: Mutex<HashMap<u64, oneshot::Sender<Incoming>>>,
}

impl Peer {
    /// Build a peer that writes serialised messages into `outgoing`.
    ///
    /// Public so a caller can drive one over a transport other than the stdio
    /// loop — and so the translation from agent events to `session/update` can
    /// be tested by reading the channel, which is the only way to assert on
    /// what an editor would actually receive.
    pub fn new(outgoing: mpsc::Sender<String>) -> Self {
        Self {
            outgoing,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn send(&self, value: &impl serde::Serialize) {
        match serde_json::to_string(value) {
            // `try_send` rather than `send().await`: this is called from the
            // agent's own turn, and a client that has stopped reading must not
            // be able to stall the turn that is trying to tell it something.
            Ok(line) => {
                if self.outgoing.try_send(line).is_err() {
                    ::tracing::warn!("acp: outgoing queue full or closed; dropped a message");
                }
            }
            Err(error) => ::tracing::error!(%error, "acp: could not serialise an outgoing message"),
        }
    }

    /// Send a `session/update` notification.
    pub fn update(&self, session_id: &str, update: SessionUpdate) {
        let params = serde_json::to_value(SessionNotification {
            session_id: session_id.to_string(),
            update,
        })
        .unwrap_or(serde_json::Value::Null);
        self.send(&Notification::new("session/update", params));
    }

    /// Reply to one of the client's requests.
    pub(crate) fn respond(&self, response: Response) {
        self.send(&response);
    }

    /// Ask the client something and wait for its answer.
    async fn request(&self, method: &str, params: serde_json::Value) -> Option<Incoming> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, tx);
        self.send(&Request::new(id, method, params));
        match rx.await {
            Ok(reply) => Some(reply),
            // The sender is dropped when the connection ends. A pending
            // question that will never be answered is not an error to report —
            // the process is going away — but it must not hang.
            Err(_) => {
                self.pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&id);
                None
            }
        }
    }

    /// Abandon every question still waiting for an answer.
    ///
    /// Called when the connection ends. Without it a turn blocked on
    /// `session/request_permission` would wait forever: the sender that would
    /// wake it lives inside this same `Peer`, which the waiting turn is holding
    /// an `Arc` to, so nothing on the outside can drop it. Dropping the senders
    /// resolves each waiter to a refusal, which is the only safe answer once
    /// there is nobody left to ask.
    pub(crate) fn disconnect(&self) {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    /// Route a reply back to whoever is waiting for it.
    pub(crate) fn resolve(&self, reply: Incoming) {
        let Some(id) = reply.id.as_ref().and_then(serde_json::Value::as_u64) else {
            ::tracing::warn!("acp: a reply arrived with no usable id");
            return;
        };
        let waiting = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id);
        match waiting {
            Some(tx) => {
                let _ = tx.send(reply);
            }
            None => ::tracing::warn!(id, "acp: a reply arrived for nothing that was asked"),
        }
    }

    /// Ask the client to authorise a tool call.
    ///
    /// Returns whether it may run. Every answer that is not an explicit allow —
    /// a rejection, a dismissed prompt, a disconnected client, an option id
    /// nobody offered — is a refusal. Permission is the one place where
    /// "I could not tell" has to mean no.
    pub async fn request_permission(
        &self,
        session_id: &str,
        tool_call_id: &str,
        title: &str,
    ) -> bool {
        let params = serde_json::to_value(RequestPermissionRequest {
            session_id: session_id.to_string(),
            tool_call: ToolCallRef {
                tool_call_id: tool_call_id.to_string(),
                title: Some(title.to_string()),
            },
            options: permission_options(),
        })
        .unwrap_or(serde_json::Value::Null);

        let Some(reply) = self.request("session/request_permission", params).await else {
            return false;
        };
        let Some(result) = reply.result else {
            return false;
        };
        let Ok(answer) = serde_json::from_value::<RequestPermissionResponse>(result) else {
            return false;
        };
        matches!(answer.selected(), Some(ALLOW_ONCE | ALLOW_ALWAYS))
    }
}

pub(crate) const ALLOW_ONCE: &str = "allow-once";
pub(crate) const ALLOW_ALWAYS: &str = "allow-always";
pub(crate) const REJECT_ONCE: &str = "reject-once";

/// What the client offers the user.
///
/// `allow_always` is offered but treated exactly as `allow_once` here: this
/// side does not yet persist a standing grant, and remembering it in memory
/// only would be a promise the next session breaks.
fn permission_options() -> Vec<PermissionOption> {
    vec![
        PermissionOption {
            option_id: ALLOW_ONCE.to_string(),
            name: "Allow once".to_string(),
            kind: PermissionOptionKind::AllowOnce,
        },
        PermissionOption {
            option_id: ALLOW_ALWAYS.to_string(),
            name: "Allow for this session".to_string(),
            kind: PermissionOptionKind::AllowAlways,
        },
        PermissionOption {
            option_id: REJECT_ONCE.to_string(),
            name: "Reject".to_string(),
            kind: PermissionOptionKind::RejectOnce,
        },
    ]
}

#[cfg(test)]
#[path = "peer_tests.rs"]
mod tests;
