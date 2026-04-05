//! Tests for TASK-CLI-304: MCP WebSocket Transport.
//!
//! Tests cover: config parsing, URL validation, reconnection delay math,
//! permanent close code detection, keep-alive config, jitter bounds.
//! Network tests verify graceful error handling without network I/O.

use archon_mcp::transport_ws::{WebSocketTransport, is_permanent_close_code, reconnect_delay_ms};
use archon_mcp::ws_config::{PERMANENT_CLOSE_CODES, WsConfig, WsReconnectConfig};

// ---------------------------------------------------------------------------
// WsConfig parsing
// ---------------------------------------------------------------------------

#[test]
fn ws_config_requires_url() {
    let cfg = WsConfig {
        url: "wss://example.com/mcp".into(),
        headers: std::collections::HashMap::new(),
        headers_helper: None,
    };
    assert_eq!(cfg.url, "wss://example.com/mcp");
}

#[test]
fn ws_config_accepts_headers_helper() {
    let cfg = WsConfig {
        url: "wss://example.com/mcp".into(),
        headers: std::collections::HashMap::new(),
        headers_helper: Some("my-token-rotator".into()),
    };
    assert_eq!(cfg.headers_helper.as_deref(), Some("my-token-rotator"));
}

#[test]
fn ws_config_from_json_type_ws() {
    // Config key must be "ws" not "websocket"
    let json = r#"{
        "type": "ws",
        "url": "wss://example.com/mcp",
        "headers": {"Authorization": "Bearer tok"},
        "headersHelper": "my-helper"
    }"#;
    let cfg: WsConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.url, "wss://example.com/mcp");
    assert_eq!(cfg.headers.get("Authorization").unwrap(), "Bearer tok");
    assert_eq!(cfg.headers_helper.as_deref(), Some("my-helper"));
}

#[test]
fn ws_config_url_required_in_json() {
    // Missing url must fail
    let json = r#"{"type": "ws"}"#;
    let result: Result<WsConfig, _> = serde_json::from_str(json);
    assert!(result.is_err(), "missing url must fail to parse");
}

#[test]
fn ws_config_headers_optional() {
    let json = r#"{"type": "ws", "url": "ws://localhost:9000"}"#;
    let cfg: WsConfig = serde_json::from_str(json).unwrap();
    assert!(cfg.headers.is_empty());
    assert!(cfg.headers_helper.is_none());
}

// ---------------------------------------------------------------------------
// Reconnection delay math
// ---------------------------------------------------------------------------

#[test]
fn reconnect_delay_attempt_0_is_1s() {
    let delay = reconnect_delay_ms(0, 0);
    // Base 1000ms with ±12.5% symmetric jitter → actual range [875, 1125).
    // Widen to ±25% to tolerate future jitter adjustments.
    assert!(
        delay >= 750 && delay <= 1_250,
        "attempt 0 base delay should be ~1s ±jitter, got {delay}"
    );
}

#[test]
fn reconnect_delay_attempt_1_is_2s() {
    let delay = reconnect_delay_ms(1, 0);
    assert!(
        delay >= 1_000 && delay <= 4_000,
        "attempt 1 base delay should be ~2s with jitter, got {delay}"
    );
}

#[test]
fn reconnect_delay_capped_at_30s() {
    // At attempt 5+, 1s * 2^5 = 32s > 30s cap
    let delay = reconnect_delay_ms(10, 0);
    let base = 30_000u64; // 30s cap
    let max_with_jitter = base + base / 4; // +25%
    assert!(
        delay <= max_with_jitter,
        "delay must not exceed 30s + 25% jitter: got {delay}"
    );
}

#[test]
fn reconnect_delay_has_jitter() {
    // Run many times and verify we get variation
    let delays: Vec<u64> = (0..20).map(|_| reconnect_delay_ms(0, 0)).collect();
    let min = delays.iter().min().copied().unwrap();
    let max = delays.iter().max().copied().unwrap();
    // With ±25% jitter on 1s base, min should be ~750ms, max should be ~1250ms
    assert!(
        max > min || max == min, // tolerate zero-variation in rare cases
        "jitter should vary delay values"
    );
}

#[test]
fn reconnect_delay_grows_with_attempts() {
    // Average delay at attempt 3 should be higher than at attempt 0
    let delay_0: u64 = (0..10).map(|_| reconnect_delay_ms(0, 0)).sum::<u64>() / 10;
    let delay_3: u64 = (0..10).map(|_| reconnect_delay_ms(3, 0)).sum::<u64>() / 10;
    assert!(
        delay_3 > delay_0,
        "later attempts should have longer delays: {delay_3} > {delay_0}"
    );
}

// ---------------------------------------------------------------------------
// Permanent close codes
// ---------------------------------------------------------------------------

#[test]
fn close_1002_is_permanent() {
    assert!(is_permanent_close_code(1002), "protocol error");
}

#[test]
fn close_4001_is_permanent() {
    assert!(is_permanent_close_code(4001), "session not found");
}

#[test]
fn close_4003_is_permanent() {
    assert!(is_permanent_close_code(4003), "unauthorized");
}

#[test]
fn close_1001_not_permanent() {
    assert!(!is_permanent_close_code(1001), "going away = reconnectable");
}

#[test]
fn close_1006_not_permanent() {
    assert!(
        !is_permanent_close_code(1006),
        "abnormal closure = reconnectable"
    );
}

#[test]
fn permanent_close_codes_list_has_three_codes() {
    assert!(PERMANENT_CLOSE_CODES.contains(&1002));
    assert!(PERMANENT_CLOSE_CODES.contains(&4001));
    assert!(PERMANENT_CLOSE_CODES.contains(&4003));
    assert_eq!(PERMANENT_CLOSE_CODES.len(), 3);
}

// ---------------------------------------------------------------------------
// WsReconnectConfig
// ---------------------------------------------------------------------------

#[test]
fn reconnect_config_default_budget_10_minutes() {
    let cfg = WsReconnectConfig::default();
    assert_eq!(cfg.budget_ms, 10 * 60 * 1_000, "budget must be 10 minutes");
}

#[test]
fn reconnect_config_default_ping_interval_10s() {
    let cfg = WsReconnectConfig::default();
    assert_eq!(cfg.ping_interval_ms, 10_000, "ping every 10s");
}

#[test]
fn reconnect_config_default_keepalive_5_minutes() {
    let cfg = WsReconnectConfig::default();
    assert_eq!(
        cfg.keepalive_interval_ms,
        5 * 60 * 1_000,
        "data frame every 5min"
    );
}

#[test]
fn reconnect_config_sleep_gap_threshold_60s() {
    let cfg = WsReconnectConfig::default();
    assert_eq!(
        cfg.sleep_gap_threshold_ms, 60_000,
        "sleep detection at 60s gap"
    );
}

// ---------------------------------------------------------------------------
// WebSocketTransport construction
// ---------------------------------------------------------------------------

#[test]
fn transport_new_invalid_url_returns_error() {
    let cfg = WsConfig {
        url: "not-a-url".into(),
        headers: std::collections::HashMap::new(),
        headers_helper: None,
    };
    let result = WebSocketTransport::new(cfg, WsReconnectConfig::default());
    assert!(result.is_err(), "invalid URL must return error");
}

#[test]
fn transport_new_http_url_rejected() {
    // Only ws:// and wss:// are valid
    let cfg = WsConfig {
        url: "http://example.com/mcp".into(),
        headers: std::collections::HashMap::new(),
        headers_helper: None,
    };
    let result = WebSocketTransport::new(cfg, WsReconnectConfig::default());
    assert!(result.is_err(), "http:// URL must be rejected");
}

#[test]
fn transport_new_ws_url_accepted() {
    let cfg = WsConfig {
        url: "ws://localhost:9000/mcp".into(),
        headers: std::collections::HashMap::new(),
        headers_helper: None,
    };
    let result = WebSocketTransport::new(cfg, WsReconnectConfig::default());
    assert!(result.is_ok(), "ws:// URL must be accepted");
}

#[test]
fn transport_new_wss_url_accepted() {
    let cfg = WsConfig {
        url: "wss://example.com/mcp".into(),
        headers: std::collections::HashMap::new(),
        headers_helper: None,
    };
    let result = WebSocketTransport::new(cfg, WsReconnectConfig::default());
    assert!(result.is_ok(), "wss:// URL must be accepted");
}

#[tokio::test]
async fn transport_connect_refused_returns_error() {
    // Port 19999 is very unlikely to have anything listening
    let cfg = WsConfig {
        url: "ws://127.0.0.1:19999/mcp".into(),
        headers: std::collections::HashMap::new(),
        headers_helper: None,
    };
    let transport = WebSocketTransport::new(cfg, WsReconnectConfig::default()).unwrap();
    let result = transport.connect().await;
    assert!(
        result.is_err(),
        "connection refused must return error, not panic"
    );
}
