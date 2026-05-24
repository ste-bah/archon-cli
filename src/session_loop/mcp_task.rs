//! MCP lifecycle task — channel-based decoupling (Path #1).
//!
//! Motivation (TASK-SESSION-LOOP-EXTRACT, after Path #3 fallback):
//! calling `McpServerManager::restart_server` / `enable_server`
//! inline inside `run_session_loop`'s boxed future produced a
//! transitive non-Send `CoerceUnsized` error — both methods
//! internally call `connect::connect_server()` at
//! `crates/archon-mcp/src/lifecycle/mod.rs:134,187`, and some
//! transitive transport-layer state (tungstenite / rmcp /
//! sse_transport handshake) is not `Send`. Named async fn wrappers
//! (Path #3) did not clear it — so this module applies the textbook
//! pattern: a dedicated OS thread with its own current-thread tokio
//! runtime owns the non-Send work exclusively, and the session
//! loop communicates with it via owned `String` request messages
//! + `oneshot` reply channels. The session loop's top-level future
//! stays Send because the non-Send state never crosses the
//! `tokio::spawn` boundary.
//!
//! Why a dedicated thread (not `tokio::task::spawn_local` on a
//! `LocalSet`): a `LocalSet` must run on the caller thread, which
//! would require restructuring the whole session bootstrap to
//! host the LocalSet. A dedicated OS thread with its own runtime
//! is self-contained — the channel handle is `Send + 'static` and
//! can be captured by any async context, including the session
//! loop's boxed future.
//!
//! The non-Send transport futures live entirely INSIDE this
//! thread's runtime. They never cross the channel; only owned
//! request enums and reply values do, all of which are `Send`.

use archon_mcp::lifecycle::McpServerManager;
use archon_mcp::types::McpError;
use archon_tui::app::{McpServerEntry, TuiEvent};
use tokio::sync::{mpsc, oneshot};

/// Request message — owned `String` so the struct is `Send`.
#[derive(Debug)]
pub(crate) enum McpLifecycleRequest {
    Restart(String),
    Enable(String),
    Disable(String),
}

/// Reply envelope — `McpError` is `Send`, `Result<(), McpError>` is `Send`.
#[derive(Debug)]
pub(crate) struct McpLifecycleReply {
    pub result: Result<(), McpError>,
}

/// Channel handle returned by [`spawn_mcp_lifecycle_task`].
///
/// Both the `Sender` and the `(Request, oneshot::Sender<Reply>)` tuple
/// it carries are `Send + 'static`, so the handle crosses the
/// `tokio::spawn` boundary freely.
pub(crate) type McpLifecycleTx =
    mpsc::UnboundedSender<(McpLifecycleRequest, oneshot::Sender<McpLifecycleReply>)>;

/// Spawn a dedicated OS thread that owns `mgr` and runs lifecycle
/// operations on its own current-thread tokio runtime.
///
/// The returned sender is the ONLY way to reach `mgr.restart_server` /
/// `enable_server` / `disable_server` from the session loop. All
/// three operations are routed through the channel so that their
/// transitively non-Send futures never touch the session-loop
/// spawn boundary.
///
/// `mgr` must be owned by the task — pass a clone (`McpServerManager`
/// is `#[derive(Clone)]` with `Arc<RwLock<_>>` internals, so the
/// session still sees the same underlying state).
pub(crate) fn spawn_mcp_lifecycle_task(mgr: McpServerManager) -> McpLifecycleTx {
    let (tx, mut rx) =
        mpsc::unbounded_channel::<(McpLifecycleRequest, oneshot::Sender<McpLifecycleReply>)>();

    std::thread::Builder::new()
        .name("archon-mcp-lifecycle".into())
        .spawn(move || {
            // Current-thread runtime — no work-stealing, no Send
            // requirement on spawned futures. Everything runs on
            // THIS thread, so the non-Send transport state is
            // confined here forever.
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "mcp-lifecycle: failed to build current-thread runtime; \
                         MCP reconnect/enable/disable will silently fail"
                    );
                    return;
                }
            };

            rt.block_on(async move {
                while let Some((req, reply_tx)) = rx.recv().await {
                    let result = match req {
                        McpLifecycleRequest::Restart(name) => mgr.restart_server(&name).await,
                        McpLifecycleRequest::Enable(name) => mgr.enable_server(&name).await,
                        McpLifecycleRequest::Disable(name) => mgr.disable_server(&name).await,
                    };
                    // Receiver may have dropped (session loop ended).
                    // That's fine — just discard the reply.
                    let _ = reply_tx.send(McpLifecycleReply { result });
                }
                tracing::debug!("mcp-lifecycle: request channel closed; shutting down");
            });
        })
        .expect("spawning archon-mcp-lifecycle OS thread should not fail");

    tx
}

/// Helper used by `run_session_loop` to route a restart through the
/// channel and await the reply. Returns `Ok(())` on channel errors
/// so callers get the same "fire-and-forget-ish" ergonomics as the
/// old inline `let _ = mgr.restart_server(..).await;`.
pub(super) async fn request_restart(tx: &McpLifecycleTx, name: &str) -> Result<(), McpError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    if tx
        .send((McpLifecycleRequest::Restart(name.to_string()), reply_tx))
        .is_err()
    {
        return Ok(()); // lifecycle thread gone; nothing to do
    }
    match reply_rx.await {
        Ok(reply) => reply.result,
        Err(_) => Ok(()), // sender dropped; treat as no-op
    }
}

/// Channel-routed `enable_server` — see [`request_restart`].
pub(super) async fn request_enable(tx: &McpLifecycleTx, name: &str) -> Result<(), McpError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    if tx
        .send((McpLifecycleRequest::Enable(name.to_string()), reply_tx))
        .is_err()
    {
        return Ok(());
    }
    match reply_rx.await {
        Ok(reply) => reply.result,
        Err(_) => Ok(()),
    }
}

/// Channel-routed `disable_server` — routed for symmetry / uniformity
/// even though `disable_server` does not call `connect_server` and
/// thus is not `!Send` on its own. Keeping all three through the
/// channel avoids surprising call-site mismatches and keeps the
/// session loop's spawn-boundary story trivially verifiable.
pub(super) async fn request_disable(tx: &McpLifecycleTx, name: &str) -> Result<(), McpError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    if tx
        .send((McpLifecycleRequest::Disable(name.to_string()), reply_tx))
        .is_err()
    {
        return Ok(());
    }
    match reply_rx.await {
        Ok(reply) => reply.result,
        Err(_) => Ok(()),
    }
}

pub(super) async fn handle_overlay_action(
    rest: &str,
    mcp_lifecycle_tx: &McpLifecycleTx,
    mcp_manager: &McpServerManager,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
) {
    let parts: Vec<&str> = rest.trim().splitn(2, ' ').collect();
    if parts.len() == 2 {
        let (server_name, action) = (parts[0], parts[1]);
        match action {
            "reconnect" => {
                let _ = request_restart(mcp_lifecycle_tx, server_name).await;
            }
            "disable" => {
                let _ = request_disable(mcp_lifecycle_tx, server_name).await;
            }
            "enable" => {
                let _ = request_enable(mcp_lifecycle_tx, server_name).await;
            }
            _ => {}
        }
        let updated = refreshed_server_entries(mcp_manager).await;
        let _ = input_tui_tx.send(TuiEvent::UpdateMcpManager(updated));
    }
    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
}

async fn refreshed_server_entries(mcp_manager: &McpServerManager) -> Vec<McpServerEntry> {
    let mut updated = Vec::new();
    for (name, state, disabled) in mcp_manager.get_server_info().await {
        let state_str = if disabled {
            "disabled"
        } else {
            match state {
                archon_mcp::types::ServerState::Ready => "ready",
                archon_mcp::types::ServerState::Starting
                | archon_mcp::types::ServerState::Restarting => "starting",
                archon_mcp::types::ServerState::Crashed => "crashed",
                archon_mcp::types::ServerState::Stopped => "stopped",
            }
        };
        let tools = if state_str == "ready" {
            mcp_manager.list_tools_for(&name).await
        } else {
            Vec::new()
        };
        updated.push(McpServerEntry {
            name,
            state: state_str.to_string(),
            tool_count: tools.len(),
            disabled,
            tools,
        });
    }
    updated
}
