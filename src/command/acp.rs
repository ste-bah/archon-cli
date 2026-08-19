//! `archon acp` — drive archon from an ACP-capable editor (#189 Phase 11).
//!
//! The protocol lives in `archon-acp`, which knows nothing about this agent.
//! This is the other half: one `AcpAgent` implementation that turns a client's
//! prompt into a real turn, and the agent's event stream back into the
//! `session/update` notifications the client renders.
//!
//! Everything an editor sees comes from the same event stream the TUI reads,
//! so an ACP client and a terminal session cannot end up with different ideas
//! of what happened.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use archon_acp::protocol::StopReason;
use archon_acp::{
    AcpAgent, ContentBlock, Peer, SessionUpdate, ToolCallContent, ToolKind, ToolStatus,
};
use archon_core::agent::{Agent, AgentEvent, TimestampedEvent};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// The agent behind one stdio connection.
///
/// A single session: an editor gets one archon per connection, which is what
/// `archon acp` is — a process the editor spawns and owns.
pub(crate) struct ArchonAcpAgent {
    session_id: String,
    agent: Arc<Mutex<Agent>>,
    events: Arc<Mutex<tokio::sync::mpsc::Receiver<TimestampedEvent>>>,
    /// Replaced per turn. Cancelling this one is what stops the turn in flight.
    cancel: Arc<std::sync::Mutex<CancellationToken>>,
}

impl ArchonAcpAgent {
    pub(crate) fn new(
        session_id: String,
        agent: Agent,
        events: tokio::sync::mpsc::Receiver<TimestampedEvent>,
    ) -> Self {
        Self {
            session_id,
            agent: Arc::new(Mutex::new(agent)),
            events: Arc::new(Mutex::new(events)),
            cancel: Arc::new(std::sync::Mutex::new(CancellationToken::new())),
        }
    }
}

#[async_trait::async_trait]
impl AcpAgent for ArchonAcpAgent {
    /// The session already exists — this process *is* the session.
    ///
    /// `cwd` is not honoured by re-rooting: the working directory was fixed
    /// when the process started, and silently moving it would make every path
    /// the editor has already shown the user wrong. A client that wants a
    /// different root spawns a different process, which is how ACP is meant to
    /// be used.
    async fn new_session(&self, cwd: &str) -> anyhow::Result<String> {
        let actual = std::env::current_dir().unwrap_or_default();
        if !cwd.is_empty() && std::path::Path::new(cwd) != actual {
            ::tracing::warn!(
                requested = cwd,
                actual = %actual.display(),
                "acp: the client asked for a different working directory; \
                 this process is already rooted elsewhere"
            );
        }
        Ok(self.session_id.clone())
    }

    async fn prompt(&self, session_id: &str, text: &str, peer: Arc<Peer>) -> StopReason {
        let token = CancellationToken::new();
        *self
            .cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = token.clone();

        let mut agent = self.agent.lock().await;
        agent.set_cancel_token(Some(token.clone()));
        let mut events = self.events.lock().await;

        let turn = agent.process_message(text);
        tokio::pin!(turn);

        // Drain and forward while the turn runs. Both halves are polled
        // together rather than in sequence, because the events are what the
        // editor renders *during* the turn — collecting them afterwards would
        // make streaming output arrive all at once at the end.
        let outcome = loop {
            tokio::select! {
                result = &mut turn => break result,
                Some(event) = events.recv() => {
                    forward(&peer, session_id, event.inner);
                }
            }
        };

        // Whatever was queued as the turn finished still belongs to the client.
        while let Ok(event) = events.try_recv() {
            forward(&peer, session_id, event.inner);
        }

        if token.is_cancelled() {
            return StopReason::Cancelled;
        }
        match outcome {
            Ok(()) => StopReason::EndTurn,
            Err(error) => {
                ::tracing::error!(%error, "acp: the turn failed");
                peer.update(
                    session_id,
                    SessionUpdate::AgentMessageChunk {
                        content: ContentBlock::text(format!("archon: {error}")),
                    },
                );
                StopReason::Refusal
            }
        }
    }

    fn cancel(&self, _session_id: &str) {
        self.cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancel();
    }
}

/// Translate one agent event into what the client renders.
///
/// Events with no ACP counterpart are dropped rather than forced into
/// `agent_message_chunk`: an editor showing context-pressure telemetry as
/// assistant prose would be worse than not showing it.
fn forward(peer: &Peer, session_id: &str, event: AgentEvent) {
    match event {
        AgentEvent::TextDelta(text) => peer.update(
            session_id,
            SessionUpdate::AgentMessageChunk {
                content: ContentBlock::text(text),
            },
        ),
        // Thinking is its own update kind, so a client can fold it away. The
        // transient preview is deliberately not forwarded — it exists to be
        // revised, and an editor has nowhere to un-render it.
        AgentEvent::ThinkingDelta(text) => peer.update(
            session_id,
            SessionUpdate::AgentThoughtChunk {
                content: ContentBlock::text(text),
            },
        ),
        AgentEvent::ToolCallStarted { name, id } => peer.update(
            session_id,
            SessionUpdate::ToolCall {
                title: name.clone(),
                kind: ToolKind::for_tool(&name),
                tool_call_id: id,
                status: ToolStatus::InProgress,
            },
        ),
        AgentEvent::ToolCallComplete { id, result, .. } => peer.update(
            session_id,
            SessionUpdate::ToolCallUpdate {
                tool_call_id: id,
                status: if result.is_error {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Completed
                },
                content: vec![ToolCallContent::text(truncate(&result.content))],
            },
        ),
        _ => {}
    }
}

/// Longest tool result sent to a client.
///
/// An editor renders this inline next to the call, so a whole build log would
/// bury the conversation. The full output is still in the agent's own context;
/// this is the summary line beside the call.
const MAX_TOOL_RESULT_CHARS: usize = 2_000;

fn truncate(content: &str) -> String {
    if content.chars().count() <= MAX_TOOL_RESULT_CHARS {
        return content.to_string();
    }
    let kept: String = content.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    format!("{kept}\n[truncated for display]")
}

/// How long to wait for the sandbox audit writer to flush on shutdown.
/// Matches the interactive, headless and IDE session paths.
const AUDIT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Boot a session and serve ACP on stdin/stdout until the client disconnects.
pub(crate) async fn handle_acp_command(
    workspace: Option<PathBuf>,
    cli: &crate::cli_args::Cli,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
) -> anyhow::Result<()> {
    if let Some(root) = workspace.as_deref() {
        // Same reasoning as `archon ide-stdio`: the agent's working directory,
        // its per-project stores under `.archon/`, and the project context in
        // the system prompt all derive from the process cwd, and an editor is
        // free to spawn a helper process anywhere.
        std::env::set_current_dir(root)
            .map_err(|error| anyhow::anyhow!("could not enter {}: {error}", root.display()))?;
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let crate::session::BuiltAgent {
        agent,
        event_rx,
        sandbox_audit_drain,
        ..
    } = crate::session::build_agent::build_session_agent(
        config,
        &session_id,
        cli,
        env_vars,
        resolved_flags,
        false,
    )
    .await
    .map_err(|exit_code| anyhow::anyhow!("ACP session bootstrap failed (exit code {exit_code})"))?;

    ::tracing::info!(%session_id, "acp: serving on stdio");
    let acp_agent: Arc<dyn AcpAgent> = Arc::new(ArchonAcpAgent::new(session_id, agent, event_rx));

    let served = archon_acp::serve(tokio::io::stdin(), tokio::io::stdout(), acp_agent).await;
    let audit = sandbox_audit_drain.shutdown(AUDIT_DRAIN_TIMEOUT).await;

    match (served, audit) {
        (Ok(()), Ok(_)) => Ok(()),
        // A transport failure is the one an editor can act on, so it wins; the
        // audit failure is still surfaced rather than swallowed.
        (Err(served_error), Ok(_)) => Err(served_error),
        (Ok(()), Err(audit_error)) => Err(audit_error),
        (Err(served_error), Err(audit_error)) => Err(anyhow::anyhow!(
            "ACP loop failed: {served_error:#}; sandbox audit drain failed: {audit_error:#}"
        )),
    }
}

#[cfg(test)]
#[path = "acp_tests.rs"]
mod tests;
