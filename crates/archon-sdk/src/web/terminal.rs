//! Browser terminal pane: a real `archon` process under a PTY, streamed to
//! xterm.js over a WebSocket.
//!
//! This is the only surface in the workbench that is not an inspector or a
//! named, auditable action. It is arbitrary code execution on the host under
//! the server's own credentials, so the gate is in [`is_available`] and is
//! applied at *router construction* — see [`super::server::build_app`]. When it
//! says no, the route is never registered and the path 404s like any other
//! unknown URL. That is deliberate: a handler that checks and returns 403 is
//! one refactor away from checking the wrong thing, and it advertises that a
//! shell is there to be unlocked.
//!
//! Note also what this is *not*: it does not attach to the session you are
//! already sitting in. Nothing here can, because archon does not own the
//! terminal it was started in — your shell does. This spawns a new process.

use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderMap, StatusCode,
        header::{ORIGIN, SEC_WEBSOCKET_PROTOCOL},
    },
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{CommandBuilder, PtySize};
use serde::Deserialize;

use super::terminal_pty::PtySession;
use super::{AppState, WebConfig, api::EffectivePolicySummary};

/// Sub-protocol carrying the bearer token when one is configured.
///
/// The browser `WebSocket` constructor cannot set an `Authorization` header, so
/// the token rides the one header it *can* influence. It is not put in the
/// query string, which would land in access logs and `Referer` headers — the
/// same reasoning that keeps `check_auth` header-only for the REST API.
const BEARER_PROTOCOL_PREFIX: &str = "archon.bearer.";

/// Fallback size used until the first `resize`. The frontend reports the fitted
/// size in the query string on connect, so this only applies to clients that do
/// not, and 80x24 is the size every terminal has agreed to fall back to.
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// Is the terminal route allowed to exist in this process at all?
///
/// Both conditions are hard requirements, not defaults:
///
/// 1. **Loopback bind, unconditionally.** A bearer token is an adequate
///    credential for a read-only API. It is not what anyone would choose to
///    stand between the open internet and a shell, and the failure mode of a
///    leaked token is total. So a non-loopback bind removes the route outright
///    — including under
///    `unsafe_allow_unauthenticated_nonlocal_bind_for_cli`, which exists to put
///    a *read-only* workbench on a trusted network and must not silently
///    upgrade into remote code execution. That flag is not consulted here, and
///    that omission is the point.
/// 2. **Its own policy flag.** `allow_web_terminal`, never
///    `allow_mutating_actions`. Ingesting a document and running arbitrary
///    commands are not the same risk; folding them into one switch means an
///    operator who wanted uploads gets a shell they did not ask for.
pub(super) fn is_available(config: &WebConfig, policy: &EffectivePolicySummary) -> bool {
    config.is_localhost() && policy.web.allow_web_terminal
}

#[derive(Debug, Deserialize)]
pub(crate) struct TerminalQuery {
    cols: Option<u16>,
    rows: Option<u16>,
}

/// Control frame from the browser. Input bytes travel as binary frames, so the
/// text channel is free to carry structured control messages without an
/// in-band escape scheme that a paste could collide with.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum TerminalControl {
    Resize { cols: u16, rows: u16 },
}

pub(crate) async fn ws_handler(
    State(state): State<AppState>,
    Query(query): Query<TerminalQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // WebSocket upgrades are exempt from the same-origin policy: any page on
    // the internet can open one to 127.0.0.1 and the browser will happily
    // attach the connection. CORS does not apply. So the `Origin` header is
    // checked here by hand — without it, "loopback only" would still leave the
    // shell reachable from any tab the operator has open.
    if !origin_is_local(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            "terminal: cross-origin upgrade refused",
        )
            .into_response();
    }

    let selected_protocol = match authorize(&state, &headers) {
        Ok(protocol) => protocol,
        Err(response) => return response,
    };

    let cwd = state.paths.cwd.clone();
    let size = PtySize {
        cols: query.cols.unwrap_or(DEFAULT_COLS).max(1),
        rows: query.rows.unwrap_or(DEFAULT_ROWS).max(1),
        pixel_width: 0,
        pixel_height: 0,
    };

    let ws = match selected_protocol {
        // Echoing the offered sub-protocol back is required: a browser that
        // offered one and got nothing selected fails the handshake.
        Some(protocol) => ws.protocols([protocol]),
        None => ws,
    };
    ws.on_upgrade(move |socket| async move {
        if let Err(error) = run_session(socket, cwd, size).await {
            tracing::warn!(%error, "terminal: session ended with an error");
        }
    })
}

/// Bearer check for the upgrade, mirroring [`super::check_auth`] but reading
/// the sub-protocol as well, because browsers cannot set request headers here.
fn authorize(state: &AppState, headers: &HeaderMap) -> Result<Option<String>, Response> {
    let Some(ref required) = state.token else {
        return Ok(None);
    };

    let header_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if header_token.is_some_and(|token| archon_core::remote::auth::validate_token(required, token))
    {
        return Ok(None);
    }

    let offered = offered_protocols(headers);
    let matched = offered.iter().find(|protocol| {
        protocol
            .strip_prefix(BEARER_PROTOCOL_PREFIX)
            .is_some_and(|token| archon_core::remote::auth::validate_token(required, token))
    });
    match matched {
        Some(protocol) => Ok(Some(protocol.clone())),
        None => Err((StatusCode::UNAUTHORIZED, "Unauthorized").into_response()),
    }
}

fn offered_protocols(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

/// A missing `Origin` means the caller is not a browser — every browser sends
/// one on a WebSocket handshake — and a non-browser client on loopback already
/// has whatever the shell would give it. Anything else must match this server's
/// own origin, which also blocks DNS-rebinding: a rebound name resolves to
/// 127.0.0.1 but the page's origin is still the attacker's host.
fn origin_is_local(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|value| value.to_str().ok()) else {
        return true;
    };
    let port = state.api.config().web.port;
    [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        format!("http://[::1]:{port}"),
    ]
    .iter()
    .any(|allowed| allowed == origin)
}

async fn run_session(
    socket: WebSocket,
    cwd: std::path::PathBuf,
    size: PtySize,
) -> anyhow::Result<()> {
    let mut session = PtySession::spawn(archon_command(&cwd)?, size)?;
    tracing::info!(pid = ?session.child_pid(), "terminal: session started");

    // Split so output can be written while input is still being awaited; a
    // single `WebSocket` cannot be borrowed mutably by two `select!` arms.
    let (mut sink, mut stream) = socket.split();

    loop {
        tokio::select! {
            chunk = session.next_output() => {
                let Some(chunk) = chunk else {
                    // Child exited or closed the PTY. Nothing left to relay.
                    break;
                };
                if sink.send(Message::Binary(chunk.into())).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(message)) => {
                        if !handle_client_message(&session, message) {
                            break;
                        }
                    }
                    // Both a protocol error and a closed socket mean the tab is
                    // gone; `session` drops below either way, killing the child.
                    Some(Err(_)) | None => break,
                }
            }
        }
    }

    let _ = sink.close().await;
    Ok(())
}

/// Returns `false` when the connection should end.
fn handle_client_message(session: &PtySession, message: Message) -> bool {
    match message {
        Message::Binary(bytes) => {
            session.send_input(bytes.to_vec());
            true
        }
        Message::Text(text) => {
            match serde_json::from_str::<TerminalControl>(text.as_str()) {
                Ok(TerminalControl::Resize { cols, rows }) => session.resize(rows, cols),
                Err(error) => tracing::debug!(%error, "terminal: unparsed control frame"),
            }
            true
        }
        Message::Close(_) => false,
        // axum answers pings itself; nothing here needs the pong.
        Message::Ping(_) | Message::Pong(_) => true,
    }
}

/// The command to run in the PTY: this same binary, no arguments, which is the
/// interactive TUI. `current_exe` rather than a `PATH` lookup so a workbench
/// served by a development build spawns that build and not whatever `archon` a
/// stale install left on `PATH`.
fn archon_command(cwd: &std::path::Path) -> anyhow::Result<CommandBuilder> {
    let exe = std::env::current_exe()
        .map_err(|error| anyhow::anyhow!("terminal: cannot resolve archon binary: {error}"))?;
    let mut command = CommandBuilder::new(exe);
    command.cwd(cwd);
    // ConPTY and openpty both render colour, and the TUI picks its palette from
    // TERM. Without this the inherited value may be `dumb` or unset, which
    // ratatui reads as "no styling".
    command.env("TERM", "xterm-256color");
    Ok(command)
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
