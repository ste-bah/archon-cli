//! OAuth 2.0 / PKCE-wrapped MCP SSE transport (#182 TASK-P0-B.2b).
//!
//! Builds on the classic SSE transport from #197 (`crate::sse_mcp_transport`)
//! by replacing the outbound POST pump with an auth-aware version:
//!
//! * Initial `GET /sse` carries `Authorization: Bearer <current>` from the
//!   [`OAuthClient`].
//! * Every outbound `POST /message` carries the **current** bearer — read
//!   fresh per-send so refreshes propagate without restarting the session.
//! * On a `401 Unauthorized` response, the pump calls
//!   [`OAuthClient::refresh`], updates the shared token state, and retries
//!   the POST **once** with the new bearer.
//! * On a second failure (or refresh itself failing) the message is dropped
//!   with a `tracing::warn!`; rmcp observes the missing response as a request
//!   timeout. **There is no infinite retry.**
//!
//! Deliberately out-of-scope for this ticket (consistent with #197 D2):
//!   * Reconnecting the SSE GET stream if the server drops it after token
//!     expiry — deferred to #202 MCP-SSE-HARDEN-RETRY.
//!   * Proactive token-expiry prediction (refresh before server rejects) —
//!     tracked under #202.

use std::collections::HashMap;
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use rmcp::service::{RoleClient, TxJsonRpcMessage};
use tokio::sync::mpsc;
use url::Url;

use crate::client::McpClient;
use crate::oauth_pkce::OAuthClient;
use crate::sse_mcp_transport::{
    SSE_CHANNEL_BUFFER, SseMcpSink, build_inbound_stream, setup_sse_inbound,
};
use crate::types::{McpError, ServerConfig};

/// Open a fully-authenticated MCP connection:
///   1. Seed initial `Authorization: Bearer <current_token>` on GET /sse.
///   2. Discover endpoint via `event: endpoint`.
///   3. Wire up an OAuth-aware POST pump (auto-refresh on 401, bounded retry).
///   4. Run `McpClient::initialize` on the resulting transport.
///
/// `oauth` MUST already hold valid tokens — typically obtained via
/// [`OAuthClient::exchange_code`] after driving the `/authorize` flow.
/// `OAuthClient` is `Clone`; this function clones it into the background
/// POST pump so refreshes are visible to callers that keep their own handle.
pub async fn connect_mcp_with_oauth(
    sse_url: &str,
    oauth: OAuthClient,
    connect_timeout: Duration,
) -> Result<McpClient, McpError> {
    // Seed the initial Authorization header — the SSE GET stream itself is
    // auth-gated on most IdPs.
    let initial_token = oauth.access_token().await;
    let mut initial_headers: HashMap<String, String> = HashMap::new();
    initial_headers.insert(
        "Authorization".to_string(),
        format!("Bearer {initial_token}"),
    );

    let setup = setup_sse_inbound(sse_url, Some(&initial_headers), connect_timeout).await?;

    let rx_stream = build_inbound_stream(setup.frame_rx);

    // User-supplied headers (none in this minimal-viable signature) would go
    // through `extra_headers`. The per-POST Bearer is injected fresh inside
    // the pump task; strip any stale Authorization header from the extras.
    let extra_headers: HashMap<HeaderName, HeaderValue> = setup
        .header_map
        .into_iter()
        .filter(|(n, _)| !n.as_str().eq_ignore_ascii_case("authorization"))
        .collect();

    let (tx_chan, tx_rx) = mpsc::channel::<TxJsonRpcMessage<RoleClient>>(SSE_CHANNEL_BUFFER);
    tokio::spawn(oauth_post_pump_task(
        setup.http_client,
        setup.post_url,
        extra_headers,
        oauth,
        tx_rx,
    ));

    let sink = futures_util::sink::unfold(
        tx_chan,
        |tx, msg: TxJsonRpcMessage<RoleClient>| async move {
            tx.send(msg).await.map_err(|e| {
                McpError::Transport(format!("oauth-sse: outbound channel closed: {e}"))
            })?;
            Ok::<_, McpError>(tx)
        },
    );
    let sink_boxed: SseMcpSink = Box::pin(sink);

    // Synthesize a minimal ServerConfig for rmcp's init handshake. The
    // `name` field feeds log spans only; transport/url/etc are informational
    // since the raw (Sink, Stream) tuple is what rmcp actually consumes.
    let config = ServerConfig {
        name: "oauth-sse".into(),
        command: String::new(),
        args: vec![],
        env: HashMap::new(),
        disabled: false,
        transport: "sse".into(),
        url: Some(sse_url.to_string()),
        headers: None,
    };

    McpClient::initialize(&config, (sink_boxed, rx_stream)).await
}

/// Background task that drains the outbound channel and POSTs each message
/// with a fresh Bearer token. On 401, refreshes via `oauth` and retries once.
///
/// Errors past the single retry drop the message with a warn log — matches
/// the vanilla `post_pump_task` behavior so rmcp surfaces missing responses
/// as request-level timeouts rather than transport-level errors.
async fn oauth_post_pump_task(
    client: reqwest::Client,
    url: Url,
    extra_headers: HashMap<HeaderName, HeaderValue>,
    oauth: OAuthClient,
    mut rx: mpsc::Receiver<TxJsonRpcMessage<RoleClient>>,
) {
    while let Some(msg) = rx.recv().await {
        let body = match serde_json::to_string(&msg) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "oauth-sse: failed to serialize outgoing JSON-RPC");
                continue;
            }
        };

        // ── First attempt — current bearer ──────────────────────────────
        let token = oauth.access_token().await;
        let first = post_with_bearer(&client, &url, &extra_headers, &token, &body).await;

        let should_retry = match &first {
            Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED => true,
            Ok(resp) if resp.status().is_success() => {
                tracing::trace!(status = %resp.status(), "oauth-sse: POST ok");
                false
            }
            Ok(resp) => {
                tracing::warn!(status = %resp.status(), "oauth-sse: POST non-2xx (non-401)");
                false
            }
            Err(e) => {
                tracing::warn!(error = %e, "oauth-sse: POST send failed");
                false
            }
        };
        drop(first);

        if !should_retry {
            continue;
        }

        // ── 401 path: refresh + retry exactly once ──────────────────────
        tracing::info!("oauth-sse: POST returned 401, attempting token refresh");
        let new_token = match oauth.refresh().await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "oauth-sse: refresh failed, dropping message (no retry loop)"
                );
                continue;
            }
        };

        match post_with_bearer(&client, &url, &extra_headers, &new_token, &body).await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(status = %resp.status(), "oauth-sse: retry after refresh OK");
            }
            Ok(resp) => {
                tracing::warn!(
                    status = %resp.status(),
                    "oauth-sse: retry after refresh still non-2xx, dropping message"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "oauth-sse: retry after refresh send error, dropping message"
                );
            }
        }
    }
    tracing::debug!("oauth-sse: outbound POST pump exited (sink closed)");
}

/// Issue a single POST with the given bearer token + extra headers.
async fn post_with_bearer(
    client: &reqwest::Client,
    url: &Url,
    extra: &HashMap<HeaderName, HeaderValue>,
    token: &str,
    body: &str,
) -> Result<reqwest::Response, reqwest::Error> {
    let mut req = client
        .post(url.as_str())
        .body(body.to_string())
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::AUTHORIZATION, format!("Bearer {token}"));
    for (name, value) in extra {
        req = req.header(name.clone(), value.clone());
    }
    req.send().await
}
