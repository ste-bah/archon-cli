//! The connection loop (#189 Phase 11).
//!
//! Reads one JSON value per line, dispatches it, and writes replies back. The
//! shape that matters is that a prompt runs on its own task: `session/cancel`
//! arrives *while* a turn is in flight, so a loop that awaited the turn inline
//! could never read the notification that stops it.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::agent::AcpAgent;
use crate::jsonrpc::{INVALID_PARAMS, Incoming, METHOD_NOT_FOUND, Response};
use crate::peer::Peer;
use crate::protocol::{
    AgentCapabilities, CancelNotification, Implementation, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PROTOCOL_VERSION, PromptCapabilities, PromptRequest, PromptResponse,
};

/// Queued outgoing lines. Bounded so a client that has stopped reading applies
/// backpressure rather than letting this process grow without limit.
const OUTGOING_QUEUE: usize = 1024;

/// Serve ACP over the given streams until the input ends.
pub async fn serve<R, W>(input: R, output: W, agent: Arc<dyn AcpAgent>) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, rx) = mpsc::channel(OUTGOING_QUEUE);
    let writer = tokio::spawn(write_lines(output, rx));
    let peer = Arc::new(Peer::new(tx));

    // In-flight requests, each on its own task. Tracked rather than detached
    // so the end of input can wait for them: a turn still running when stdin
    // closes has a reply to send, and dropping the connection first would lose
    // it. It is also what stops the writer below from waiting forever on a
    // sender one of these tasks still holds.
    let mut inflight = tokio::task::JoinSet::new();

    let mut lines = BufReader::new(input).lines();
    while let Some(line) = lines.next_line().await? {
        // Reap anything that has already finished, so a long connection does
        // not accumulate completed handles.
        while inflight.try_join_next().is_some() {}
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Incoming>(&line) {
            Ok(message) => dispatch(&peer, &agent, &mut inflight, message),
            // A malformed line has no id, so there is nothing to reply to.
            // Logged and skipped rather than ending the connection: one bad
            // message from an editor should not take the session with it.
            Err(error) => ::tracing::warn!(%error, "acp: unreadable message"),
        }
    }

    // Before waiting on them, not after: a turn blocked on a permission
    // request would otherwise wait for an answer from a client that has
    // already gone, and this loop would wait for that turn.
    peer.disconnect();
    while inflight.join_next().await.is_some() {}
    drop(peer);
    let _ = writer.await;
    Ok(())
}

async fn write_lines<W: AsyncWrite + Unpin>(mut output: W, mut rx: mpsc::Receiver<String>) {
    while let Some(line) = rx.recv().await {
        if output.write_all(line.as_bytes()).await.is_err()
            || output.write_all(b"\n").await.is_err()
            || output.flush().await.is_err()
        {
            break;
        }
    }
}

fn dispatch(
    peer: &Arc<Peer>,
    agent: &Arc<dyn AcpAgent>,
    inflight: &mut tokio::task::JoinSet<()>,
    message: Incoming,
) {
    if message.is_reply() {
        peer.resolve(message);
        return;
    }
    let Some(method) = message.method.clone() else {
        return;
    };

    if message.is_notification() {
        // Cancellation is handled inline and synchronously. It is the one
        // message whose whole value is arriving promptly, and the agent's
        // `cancel` only signals — it does not wait for the turn to notice.
        if method == "session/cancel"
            && let Ok(cancel) = serde_json::from_value::<CancelNotification>(message.params)
        {
            agent.cancel(&cancel.session_id);
        }
        return;
    }

    let Some(id) = message.id.clone() else {
        return;
    };
    let peer = Arc::clone(peer);
    let agent = Arc::clone(agent);
    // Every request is answered on its own task, so a long prompt cannot block
    // the reader that has to deliver its cancellation.
    inflight.spawn(async move {
        let response = handle(&peer, &agent, &method, message.params, id.clone()).await;
        peer.respond(response);
    });
}

async fn handle(
    peer: &Arc<Peer>,
    agent: &Arc<dyn AcpAgent>,
    method: &str,
    params: serde_json::Value,
    id: serde_json::Value,
) -> Response {
    match method {
        "initialize" => Response::ok(id, initialize_result()),
        "session/new" => match serde_json::from_value::<NewSessionRequest>(params) {
            Ok(request) => match agent.new_session(&request.cwd).await {
                Ok(session_id) => Response::ok(
                    id,
                    serde_json::to_value(NewSessionResponse { session_id })
                        .unwrap_or(serde_json::Value::Null),
                ),
                Err(error) => Response::err(id, crate::jsonrpc::INTERNAL_ERROR, error.to_string()),
            },
            Err(error) => Response::err(id, INVALID_PARAMS, error.to_string()),
        },
        "session/prompt" => match serde_json::from_value::<PromptRequest>(params) {
            Ok(request) => {
                let text = request
                    .prompt
                    .iter()
                    .filter_map(crate::protocol::ContentBlock::as_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                let stop_reason = agent
                    .prompt(&request.session_id, &text, Arc::clone(peer))
                    .await;
                Response::ok(
                    id,
                    serde_json::to_value(PromptResponse { stop_reason })
                        .unwrap_or(serde_json::Value::Null),
                )
            }
            Err(error) => Response::err(id, INVALID_PARAMS, error.to_string()),
        },
        // Answered rather than ignored. `authenticate` has nothing to do —
        // this agent runs as the user — and a client that asked deserves to
        // hear that it succeeded instead of timing out.
        "authenticate" => Response::ok(id, serde_json::json!({})),
        other => Response::err(
            id,
            METHOD_NOT_FOUND,
            format!("archon does not implement {other}"),
        ),
    }
}

fn initialize_result() -> serde_json::Value {
    serde_json::to_value(InitializeResponse {
        protocol_version: PROTOCOL_VERSION,
        agent_capabilities: AgentCapabilities {
            // Declared false because it is: `session/load` is not implemented,
            // and claiming otherwise would make a client offer resumption that
            // silently returns an empty conversation.
            load_session: false,
            prompt_capabilities: PromptCapabilities {
                image: false,
                audio: false,
                embedded_context: false,
            },
        },
        agent_info: Implementation {
            name: "archon".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        auth_methods: Vec::new(),
    })
    .unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;
