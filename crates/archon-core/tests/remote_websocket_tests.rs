use archon_core::remote::auth;
use archon_core::remote::protocol::AgentMessage;
use archon_core::remote::websocket::{WsConnectionConfig, WsServerConfig};

#[test]
fn ws_connection_config_defaults() {
    let cfg = WsConnectionConfig::default();
    assert!(
        cfg.url.contains("8420"),
        "default URL should contain port 8420, got: {}",
        cfg.url
    );
    assert!(cfg.reconnect, "reconnect should default to true");
}

#[test]
fn bearer_token_length() {
    let token = auth::generate_token();
    assert!(
        token.len() >= 64,
        "token length should be >= 64, got: {}",
        token.len()
    );
}

#[test]
fn bearer_token_uniqueness() {
    let t1 = auth::generate_token();
    let t2 = auth::generate_token();
    assert_ne!(t1, t2, "two generated tokens must differ");
}

#[test]
fn bearer_token_validation_valid() {
    assert!(
        auth::validate_token("abc", "abc"),
        "identical tokens should validate"
    );
}

#[test]
fn bearer_token_validation_invalid() {
    assert!(
        !auth::validate_token("abc", "xyz"),
        "different tokens should not validate"
    );
}

#[test]
fn bearer_token_validation_empty() {
    assert!(
        !auth::validate_token("abc", ""),
        "empty provided token should not validate"
    );
}

#[test]
fn ws_server_config_defaults() {
    let cfg = WsServerConfig::default();
    assert_eq!(cfg.port, 8420, "default port should be 8420");
}

#[test]
fn ws_connect_url_with_token() {
    let token = "mytoken123";
    let base_url = "ws://localhost:8420/ws";
    let url_with_token = format!("{}?token={}", base_url, token);
    assert!(url_with_token.contains("token=mytoken123"));
    assert!(url_with_token.starts_with("ws://"));
}

#[test]
fn ws_message_json_roundtrip() {
    let msg = AgentMessage::Ping;
    let line = msg.to_json_line().expect("serialize should succeed");
    let parsed = AgentMessage::from_json_line(&line).expect("deserialize should succeed");
    match parsed {
        AgentMessage::Ping => {}
        other => panic!("expected Ping, got {:?}", other),
    }
}

#[test]
fn ws_config_serialization() {
    let cfg = WsConnectionConfig::default();
    let json = serde_json::to_string(&cfg).expect("should serialize");
    let parsed: WsConnectionConfig = serde_json::from_str(&json).expect("should deserialize");
    assert_eq!(cfg.url, parsed.url);
    assert_eq!(cfg.reconnect, parsed.reconnect);
    assert_eq!(cfg.max_reconnect_attempts, parsed.max_reconnect_attempts);
}
