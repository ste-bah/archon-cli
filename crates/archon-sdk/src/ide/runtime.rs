//! Agent runtime behind `archon/prompt` (issue #26).
//!
//! [`IdeAgentRuntime`] owns the live [`Agent`], the outbound notification
//! queue, the permission bridge, and the in-flight turn. It is a separate type
//! from [`IdeProtocolHandler`](crate::ide::handler::IdeProtocolHandler) so the
//! handler stays a pure JSON-RPC dispatcher: with no runtime attached the
//! handler still answers every method with the correct protocol shape, which
//! is what the synchronous `StdioTransport::run` loop and the protocol shape
//! tests rely on.
//!
//! [`IdeAgentRuntime::new`] takes the `Agent` **by value** and hands back a
//! shared handle. That is the whole safety design for tools: the permission
//! channel is installed on the way through, so there is no way to attach an
//! agent to the IDE and forget it — and an agent without that channel
//! auto-approves every tool it is asked to run.

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use archon_core::agent::{Agent, TimestampedEvent};

use crate::ide::context_files::compose_prompt;
use crate::ide::events::notification_for;
use crate::ide::permission::PermissionBridge;
use crate::ide::protocol::{
    IdeError, IdePromptParams, IdeStatusResult, JRpcErrorCode, JRpcNotification,
};

/// Depth of the outbound notification queue.
///
/// Sized to the agent's own event channel on purpose. The agent already
/// blocks when its event channel fills, so matching the depth keeps a single
/// backpressure point in the system rather than adding a second one here that
/// would silently drop deltas mid-turn.
pub const IDE_NOTIFICATION_CAPACITY: usize = archon_core::agent::AGENT_EVENT_CHANNEL_CAPACITY;

/// Depth of the permission answer channel.
///
/// One. The agent asks one question at a time and blocks for the answer, so a
/// deeper queue could only ever hold answers to questions nobody asked.
const PERMISSION_CHANNEL_CAPACITY: usize = 1;

/// A prompt turn the agent is currently driving.
struct ActiveTurn {
    /// Cancels the turn. Also installed on the agent as its config-level
    /// cancel token so tool contexts see it too.
    cancel: CancellationToken,
    /// Retained only to tell "still running" from "already finished"; a
    /// finished turn must not make the next `archon/prompt` look busy.
    task: tokio::task::JoinHandle<()>,
}

/// Token and cost figures, refreshed from the agent after every turn.
///
/// `None` until the first turn ends. `archon/status` reports that absence
/// explicitly rather than answering `0`, which would be a measurement the
/// session never took.
type StatusSlot = Arc<std::sync::Mutex<Option<TurnStatus>>>;

#[derive(Clone, Copy)]
struct TurnStatus {
    input_tokens: u64,
    output_tokens: u64,
    cost: f64,
}

/// Live agent wired to the IDE protocol.
pub struct IdeAgentRuntime {
    agent: Arc<Mutex<Agent>>,
    notifications: mpsc::Sender<JRpcNotification>,
    permissions: Arc<PermissionBridge>,
    /// Session id stamped onto outbound notifications.
    ///
    /// A shared slot rather than a value because the event pump starts before
    /// the id exists: `archon/initialize` mints it, and the transport is
    /// already draining notifications by then.
    session_id: Arc<std::sync::Mutex<String>>,
    status: StatusSlot,
    /// Captured at construction, before the agent goes behind the lock, so
    /// `archon/status` and `archon/config` can answer without waiting on a
    /// turn that may be mid-stream.
    model: String,
    permission_mode: Arc<Mutex<String>>,
    active_turn: Option<ActiveTurn>,
}

impl IdeAgentRuntime {
    /// Attach `agent` to the IDE protocol and start pumping its events out as
    /// JSON-RPC notifications.
    ///
    /// `agent_events` must be the receiver paired with the sender the agent
    /// was constructed with, otherwise no deltas reach the IDE.
    ///
    /// The agent is taken by value so the permission channel can be installed
    /// before anything can run against it, and returned behind the shared
    /// handle the caller needs. This is why a tool-capable IDE session is safe
    /// now and was not in the first slice: with `permission_response_rx` set,
    /// `request_tool_permission` blocks for a real decision instead of logging
    /// "no permission channel, auto-approved" and returning `true`
    /// (`archon-core/src/agent/permission_gate.rs`).
    ///
    /// The event pump is spawned here rather than by the caller so it cannot
    /// be forgotten; that requires a Tokio runtime to be entered, which every
    /// caller (the `ide-stdio` subcommand and the tests) is.
    pub fn new(
        mut agent: Agent,
        agent_events: mpsc::Receiver<TimestampedEvent>,
    ) -> (Self, mpsc::Receiver<JRpcNotification>, Arc<Mutex<Agent>>) {
        let (permission_tx, permission_rx) = mpsc::channel(PERMISSION_CHANNEL_CAPACITY);
        agent.permission_response_rx = Some(Arc::new(Mutex::new(permission_rx)));
        let model = agent.current_model().to_string();
        let permission_mode = agent.permission_mode_handle();

        let agent = Arc::new(Mutex::new(agent));
        let (tx, rx) = mpsc::channel(IDE_NOTIFICATION_CAPACITY);
        let session_id = Arc::new(std::sync::Mutex::new(String::new()));
        let permissions = Arc::new(PermissionBridge::new(permission_tx));
        tokio::spawn(pump_agent_events(
            agent_events,
            tx.clone(),
            Arc::clone(&session_id),
            Arc::clone(&permissions),
        ));
        (
            Self {
                agent: Arc::clone(&agent),
                notifications: tx,
                permissions,
                session_id,
                status: Arc::new(std::sync::Mutex::new(None)),
                model,
                permission_mode,
                active_turn: None,
            },
            rx,
            agent,
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
            TurnContext {
                agent: Arc::clone(&self.agent),
                notifications: self.notifications.clone(),
                session_id: Arc::clone(&self.session_id),
                status: Arc::clone(&self.status),
            },
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
        // A turn abandoned mid-permission leaves the agent blocked on the
        // channel until its own timeout; retiring the request stops the IDE
        // from answering a question that no longer has a listener.
        self.permissions.close_request();
        true
    }

    /// Record what the connected client said it can do, from
    /// `archon/initialize`.
    ///
    /// Only `tool_execution` matters here, and it is load-bearing: a client
    /// with no allow/deny UI can never answer a permission prompt, so its
    /// requests are refused immediately instead of hanging until the agent's
    /// own timeout denies them anyway.
    pub fn set_client_can_approve_tools(&self, can_approve: bool) {
        self.permissions.set_client_can_answer(can_approve);
    }

    /// Deliver the user's answer to an outstanding `archon/permissionRequest`.
    pub fn respond_to_permission(&self, request_id: &str, approved: bool) -> Result<(), String> {
        self.permissions.respond(request_id, approved)
    }

    /// Session token and cost figures, or an explicit statement that there are
    /// none yet.
    pub fn status(&self) -> IdeStatusResult {
        let measured = self.status.lock().ok().and_then(|slot| *slot);
        match measured {
            Some(status) => IdeStatusResult {
                model: Some(self.model.clone()),
                input_tokens: Some(status.input_tokens),
                output_tokens: Some(status.output_tokens),
                cost: Some(status.cost),
                unavailable: None,
            },
            None => IdeStatusResult {
                model: Some(self.model.clone()),
                unavailable: Some(
                    "no turn has completed in this session yet, so there is nothing measured to report"
                        .to_string(),
                ),
                ..IdeStatusResult::default()
            },
        }
    }

    /// The model this session runs on.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Shared handle to the live permission mode, for `archon/config`.
    pub fn permission_mode(&self) -> Arc<Mutex<String>> {
        Arc::clone(&self.permission_mode)
    }
}

/// The handles one turn task needs. Grouped so `run_turn` keeps a readable
/// signature as the runtime grows.
struct TurnContext {
    agent: Arc<Mutex<Agent>>,
    notifications: mpsc::Sender<JRpcNotification>,
    session_id: Arc<std::sync::Mutex<String>>,
    status: StatusSlot,
}

/// Forward agent events to the IDE until the agent's event channel closes.
async fn pump_agent_events(
    mut agent_events: mpsc::Receiver<TimestampedEvent>,
    notifications: mpsc::Sender<JRpcNotification>,
    session_id: Arc<std::sync::Mutex<String>>,
    permissions: Arc<PermissionBridge>,
) {
    while let Some(event) = agent_events.recv().await {
        let session = current_session_id(&session_id);
        let Some(notification) = notification_for(&permissions, &session, &event.inner) else {
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
async fn run_turn(ctx: TurnContext, prompt: String, cancel: CancellationToken) {
    let mut agent = ctx.agent.lock().await;
    agent.set_cancel_token(Some(cancel.clone()));
    let outcome = tokio::select! {
        biased;
        () = cancel.cancelled() => None,
        result = agent.process_message(&prompt) => Some(result),
    };
    agent.set_cancel_token(None);
    // Read while the lock is still held: this is the one point where the
    // figures are known to belong to a finished turn rather than a partial one.
    let measured = {
        let stats = agent.session_stats.lock().await;
        TurnStatus {
            input_tokens: stats.input_tokens,
            output_tokens: stats.output_tokens,
            cost: stats.session_cost,
        }
    };
    // Published before the agent lock is released, so anything that observes
    // the session as idle also observes the figures for the turn that just
    // ended rather than the previous turn's.
    if let Ok(mut slot) = ctx.status.lock() {
        *slot = Some(measured);
    }
    drop(agent);

    match outcome {
        None => tracing::info!("IDE prompt cancelled"),
        Some(Ok(())) => {}
        Some(Err(error)) => {
            // The agent emits no event for a failed turn, so without this the
            // IDE would sit on a half-rendered reply with no completion.
            let session = current_session_id(&ctx.session_id);
            tracing::error!(%error, "IDE prompt failed");
            let params = serde_json::to_value(IdeError {
                session_id: Some(session),
                message: error.to_string(),
                code: JRpcErrorCode::INTERNAL_ERROR,
            });
            if let Ok(params) = params {
                let _ = ctx
                    .notifications
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
}
