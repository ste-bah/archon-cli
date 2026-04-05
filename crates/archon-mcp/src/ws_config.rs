//! WebSocket transport configuration for TASK-CLI-304.
//!
//! Config key in `.mcp.json` is `"ws"` (NOT `"websocket"`).
//!
//! Example:
//! ```json
//! {
//!   "mcpServers": {
//!     "remote": {
//!       "type": "ws",
//!       "url": "wss://example.com/mcp",
//!       "headers": { "Authorization": "Bearer token" },
//!       "headersHelper": "my-header-generator"
//!     }
//!   }
//! }
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Permanent close codes (reference: WebSocketTransport.ts:42-46)
// ---------------------------------------------------------------------------

/// WebSocket close codes that indicate a permanent failure — no reconnection.
///
/// - `1002` — Protocol error
/// - `4001` — Session not found
/// - `4003` — Unauthorized (triggers token rotation if `headersHelper` configured)
pub const PERMANENT_CLOSE_CODES: &[u16] = &[1002, 4001, 4003];

// ---------------------------------------------------------------------------
// WsConfig
// ---------------------------------------------------------------------------

/// Configuration for a WebSocket MCP server entry.
///
/// Parsed from `.mcp.json` entries with `"type": "ws"`.
/// The config key is `"ws"`, not `"websocket"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsConfig {
    /// WebSocket endpoint URL (`ws://` or `wss://`).
    pub url: String,

    /// Custom HTTP headers to include in the WebSocket handshake request.
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Optional identifier for a dynamic header generator function.
    ///
    /// When set and the server closes with code 4003 (unauthorized), the
    /// header generator is called to rotate credentials and reconnection is
    /// retried once before permanent failure.
    #[serde(rename = "headersHelper", default)]
    pub headers_helper: Option<String>,
}

// ---------------------------------------------------------------------------
// WsReconnectConfig
// ---------------------------------------------------------------------------

/// Reconnection strategy for `WebSocketTransport`.
///
/// Uses a time-budget model (reference: `WebSocketTransport.ts:26-27`):
/// - `DEFAULT_RECONNECT_GIVE_UP_MS = 600_000` (10 minutes)
/// - Base delay: `min(1s * 2^attempt, 30s)` with ±25% jitter
/// - Budget resets on sleep/wake gaps > 60s
#[derive(Debug, Clone)]
pub struct WsReconnectConfig {
    /// Wall-clock budget for reconnection attempts in ms. Default: 10 minutes.
    pub budget_ms: u64,

    /// WebSocket ping interval in ms. Default: 10s.
    ///
    /// If previous ping had no pong when next tick fires, connection is dead.
    pub ping_interval_ms: u64,

    /// JSON data frame keep-alive interval in ms. Default: 5 minutes.
    ///
    /// Sends `{"type":"keep_alive"}` to reset proxy idle timers (e.g. Cloudflare 5-min timeout).
    pub keepalive_interval_ms: u64,

    /// Gap between reconnect attempts (in ms) that indicates sleep/wake. Default: 60s.
    ///
    /// If the gap exceeds this threshold, budget and attempt counter are reset.
    pub sleep_gap_threshold_ms: u64,

    /// Graceful shutdown wait in ms. Default: 5s.
    ///
    /// On shutdown, send close frame and wait this long for acknowledgment.
    pub shutdown_wait_ms: u64,
}

impl Default for WsReconnectConfig {
    fn default() -> Self {
        Self {
            budget_ms: 10 * 60 * 1_000,   // 10 minutes
            ping_interval_ms: 10_000,       // 10 seconds
            keepalive_interval_ms: 5 * 60 * 1_000, // 5 minutes
            sleep_gap_threshold_ms: 60_000, // 60 seconds
            shutdown_wait_ms: 5_000,        // 5 seconds
        }
    }
}
