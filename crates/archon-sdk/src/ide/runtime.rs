//! Agent runtime behind `archon/prompt` (issue #26, first slice: read-only chat).
//!
//! [`IdeAgentRuntime`] owns the live [`Agent`], the outbound notification
//! queue, and the in-flight turn. It is a separate type from
//! [`IdeProtocolHandler`](crate::ide::handler::IdeProtocolHandler) so the
//! handler stays a pure JSON-RPC dispatcher: with no runtime attached the
//! handler still answers every method with the correct protocol shape, which
//! is what the synchronous `StdioTransport::run` loop and the protocol shape
//! tests rely on.
//!
//! Scope note: this slice deliberately carries text only. Tool execution and
//! the IDE permission round-trip are later slices, and the host must not
//! attach a tool-capable agent until the permission channel exists — see
//! [`IdeAgentRuntime::new`].

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use archon_core::agent::{Agent, TimestampedEvent};

use crate::ide::handler::event_to_notification;
use crate::ide::protocol::{IdeError, IdePromptParams, JRpcErrorCode, JRpcNotification};

/// Depth of the outbound notification queue.
///
/// Sized to the agent's own event channel on purpose. The agent already
/// blocks when its event channel fills, so matching the depth keeps a single
/// backpressure point in the system rather than adding a second one here that
/// would silently drop deltas mid-turn.
pub const IDE_NOTIFICATION_CAPACITY: usize = archon_core::agent::AGENT_EVENT_CHANNEL_CAPACITY;

/// A prompt turn the agent is currently driving.
struct ActiveTurn {
    /// Cancels the turn. Also installed on the agent as its config-level
    /// cancel token so later slices propagate it into tool contexts.
    cancel: CancellationToken,
    /// Retained only to tell "still running" from "already finished"; a
    /// finished turn must not make the next `archon/prompt` look busy.
    task: tokio::task::JoinHandle<()>,
}

/// Live agent wired to the IDE protocol.
pub struct IdeAgentRuntime {
    agent: Arc<Mutex<Agent>>,
    notifications: mpsc::Sender<JRpcNotification>,
    /// Session id stamped onto outbound notifications.
    ///
    /// A shared slot rather than a value because the event pump starts before
    /// the id exists: `archon/initialize` mints it, and the transport is
    /// already draining notifications by then.
    session_id: Arc<std::sync::Mutex<String>>,
    active_turn: Option<ActiveTurn>,
}

impl IdeAgentRuntime {
    /// Attach `agent` to the IDE protocol and start pumping its events out as
    /// JSON-RPC notifications.
    ///
    /// `agent_events` must be the receiver paired with the sender the agent
    /// was constructed with, otherwise no deltas reach the IDE.
    ///
    /// The caller is responsible for handing over an agent with **no tools**.
    /// [`Agent`] auto-approves every permission request when its
    /// `permission_response_rx` is `None` (see
    /// `archon-core/src/agent/permission_gate.rs`), and this slice has no
    /// permission round-trip, so a tool-capable agent here would run Bash and
    /// Write with nobody ever asked.
    ///
    /// Returns the runtime plus the notification receiver the transport
    /// drains. The event pump is spawned here rather than by the caller so it
    /// cannot be forgotten; that requires a Tokio runtime to be entered,
    /// which every caller (the `ide-stdio` subcommand and the tests) is.
    pub fn new(
        agent: Arc<Mutex<Agent>>,
        agent_events: mpsc::Receiver<TimestampedEvent>,
    ) -> (Self, mpsc::Receiver<JRpcNotification>) {
        let (tx, rx) = mpsc::channel(IDE_NOTIFICATION_CAPACITY);
        let session_id = Arc::new(std::sync::Mutex::new(String::new()));
        tokio::spawn(pump_agent_events(
            agent_events,
            tx.clone(),
            Arc::clone(&session_id),
        ));
        (
            Self {
                agent,
                notifications: tx,
                session_id,
                active_turn: None,
            },
            rx,
        )
    }

    /// Publish the session id that outbound notifications are tagged with.
    ///
    /// Called from `archon/initialize`; until then the pump has nothing
    /// sensible to stamp, but it also has nothing to pump.
    pub fn set_session_id(&self, session_id: &str) {
        if let Ok(mut slot) = self.session_id.lock() {
            session_id.clone_into(&mut slot);
        }
    }

    /// Start a turn for `params`, streaming its output as notifications.
    ///
    /// Returns `Err` with a human-readable reason when a turn is already in
    /// flight. Turns are serialised rather than queued: two concurrent turns
    /// would interleave their deltas on one notification stream, and the IDE
    /// has no way to tell them apart.
    pub fn start_turn(&mut self, params: &IdePromptParams) -> Result<(), String> {
        if self
            .active_turn
            .as_ref()
            .is_some_and(|turn| !turn.task.is_finished())
        {
            return Err("a prompt is already in flight for this session".to_string());
        }

        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_turn(
            Arc::clone(&self.agent),
            self.notifications.clone(),
            Arc::clone(&self.session_id),
            compose_prompt(params),
            cancel.clone(),
        ));
        self.active_turn = Some(ActiveTurn { cancel, task });
        Ok(())
    }

    /// Cancel the in-flight turn, if there is one.
    ///
    /// Returns whether a turn was actually running. The turn task observes
    /// the token and drops the `process_message` future, which stops the
    /// stream and releases the agent lock for the next prompt.
    pub fn cancel_turn(&mut self) -> bool {
        let Some(turn) = self.active_turn.as_ref() else {
            return false;
        };
        if turn.task.is_finished() {
            return false;
        }
        turn.cancel.cancel();
        true
    }
}

/// Build the text handed to the agent for one prompt.
///
/// Attached files are named rather than read: this slice has no tools, so the
/// agent cannot open them. Naming them still beats dropping them silently —
/// the model can say what it was given, and once tool execution lands the same
/// paths become directly actionable.
fn compose_prompt(params: &IdePromptParams) -> String {
    let files = params.context_files.as_deref().unwrap_or(&[]);
    if files.is_empty() {
        return params.text.clone();
    }
    let mut out = params.text.clone();
    out.push_str("\n\nFiles attached in the editor:");
    for file in files {
        out.push_str("\n- ");
        out.push_str(file);
    }
    out
}

/// Forward agent events to the IDE until the agent's event channel closes.
async fn pump_agent_events(
    mut agent_events: mpsc::Receiver<TimestampedEvent>,
    notifications: mpsc::Sender<JRpcNotification>,
    session_id: Arc<std::sync::Mutex<String>>,
) {
    while let Some(event) = agent_events.recv().await {
        let session = current_session_id(&session_id);
        let Some(notification) = event_to_notification(&session, &event.inner) else {
            continue;
        };
        if notifications.send(notification).await.is_err() {
            // Transport gone: nothing left to notify, so stop reading rather
            // than spinning against a closed channel for the rest of the turn.
            tracing::debug!("IDE notification channel closed; stopping event pump");
            return;
        }
    }
}

/// Drive one `process_message` call to completion or cancellation.
async fn run_turn(
    agent: Arc<Mutex<Agent>>,
    notifications: mpsc::Sender<JRpcNotification>,
    session_id: Arc<std::sync::Mutex<String>>,
    prompt: String,
    cancel: CancellationToken,
) {
    let mut agent = agent.lock().await;
    agent.set_cancel_token(Some(cancel.clone()));
    let outcome = tokio::select! {
        biased;
        () = cancel.cancelled() => None,
        result = agent.process_message(&prompt) => Some(result),
    };
    agent.set_cancel_token(None);
    drop(agent);

    match outcome {
        None => tracing::info!("IDE prompt cancelled"),
        Some(Ok(())) => {}
        Some(Err(error)) => {
            // The agent emits no event for a failed turn, so without this the
            // IDE would sit on a half-rendered reply with no completion.
            let session = current_session_id(&session_id);
            tracing::error!(%error, "IDE prompt failed");
            let params = serde_json::to_value(IdeError {
                session_id: Some(session),
                message: error.to_string(),
                code: JRpcErrorCode::INTERNAL_ERROR,
            });
            if let Ok(params) = params {
                let _ = notifications
                    .send(JRpcNotification {
                        jsonrpc: "2.0".to_string(),
                        method: "archon/error".to_string(),
                        params,
                    })
                    .await;
            }
        }
    }
}

fn current_session_id(slot: &Arc<std::sync::Mutex<String>>) -> String {
    slot.lock()
        .map(|value| value.clone())
        .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The issue named this as its biggest design risk: holding `Agent`
    /// behind `Arc<Mutex<_>>` only works if every field of `Agent` is
    /// `Send + Sync`. It is — pinned here so a future field cannot quietly
    /// take the option away.
    #[test]
    fn agent_can_live_behind_a_shared_mutex() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Agent>();
        assert_send_sync::<Arc<Mutex<Agent>>>();
    }

    #[test]
    fn attached_files_are_named_in_the_prompt() {
        let params = IdePromptParams {
            session_id: "s".to_string(),
            text: "explain this".to_string(),
            context_files: Some(vec!["src/main.rs".to_string()]),
        };

        let composed = compose_prompt(&params);

        assert!(composed.starts_with("explain this"));
        assert!(composed.contains("src/main.rs"));
    }

    #[test]
    fn prompt_without_attachments_is_passed_through_verbatim() {
        let params = IdePromptParams {
            session_id: "s".to_string(),
            text: "explain this".to_string(),
            context_files: None,
        };

        assert_eq!(compose_prompt(&params), "explain this");
    }
}
