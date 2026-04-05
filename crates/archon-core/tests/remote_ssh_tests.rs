use archon_core::config::{SshConfig, SshRemoteConfig};
use archon_core::headless::HeadlessRuntime;
use archon_core::remote::{protocol::AgentMessage, SyncMode};

// ---------------------------------------------------------------------------
// Protocol: JSON-line roundtrip
// ---------------------------------------------------------------------------

#[test]
fn agent_message_user_message_roundtrip() {
    let msg = AgentMessage::UserMessage { content: "hello world".to_string() };
    let line = msg.to_json_line().expect("serialize");
    assert!(line.ends_with('\n'), "JSON line must end with newline");
    let parsed = AgentMessage::from_json_line(&line).expect("deserialize");
    match parsed {
        AgentMessage::UserMessage { content } => assert_eq!(content, "hello world"),
        other => panic!("unexpected variant: {other:?}"),
    }
}

#[test]
fn agent_message_assistant_message_roundtrip() {
    let msg = AgentMessage::AssistantMessage { content: "I can help.".to_string() };
    let line = msg.to_json_line().expect("serialize");
    let parsed = AgentMessage::from_json_line(&line).expect("deserialize");
    match parsed {
        AgentMessage::AssistantMessage { content } => assert_eq!(content, "I can help."),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn agent_message_ping_roundtrip() {
    let ping = AgentMessage::Ping;
    let line = ping.to_json_line().expect("serialize");
    assert!(line.ends_with('\n'));
    let parsed = AgentMessage::from_json_line(&line).expect("deserialize");
    assert!(matches!(parsed, AgentMessage::Ping));
}

#[test]
fn agent_message_pong_roundtrip() {
    let pong = AgentMessage::Pong;
    let line = pong.to_json_line().expect("serialize");
    let parsed = AgentMessage::from_json_line(&line).expect("deserialize");
    assert!(matches!(parsed, AgentMessage::Pong));
}

#[test]
fn agent_message_tool_call_roundtrip() {
    let msg = AgentMessage::ToolCall {
        id: "call_abc123".to_string(),
        name: "bash".to_string(),
        input: serde_json::json!({"command": "ls -la"}),
    };
    let line = msg.to_json_line().expect("serialize");
    let parsed = AgentMessage::from_json_line(&line).expect("deserialize");
    match parsed {
        AgentMessage::ToolCall { id, name, .. } => {
            assert_eq!(id, "call_abc123");
            assert_eq!(name, "bash");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn agent_message_tool_result_roundtrip() {
    let msg = AgentMessage::ToolResult {
        tool_use_id: "call_abc123".to_string(),
        content: "file1.txt\nfile2.txt".to_string(),
        is_error: false,
    };
    let line = msg.to_json_line().expect("serialize");
    let parsed = AgentMessage::from_json_line(&line).expect("deserialize");
    match parsed {
        AgentMessage::ToolResult { tool_use_id, content, is_error } => {
            assert_eq!(tool_use_id, "call_abc123");
            assert_eq!(content, "file1.txt\nfile2.txt");
            assert!(!is_error);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn agent_message_tool_result_error_flag() {
    let msg = AgentMessage::ToolResult {
        tool_use_id: "call_xyz".to_string(),
        content: "permission denied".to_string(),
        is_error: true,
    };
    let line = msg.to_json_line().expect("serialize");
    let parsed = AgentMessage::from_json_line(&line).expect("deserialize");
    match parsed {
        AgentMessage::ToolResult { is_error, .. } => assert!(is_error),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn agent_message_event_roundtrip() {
    let msg = AgentMessage::Event {
        kind: "session_start".to_string(),
        data: serde_json::json!({"session_id": "abc-123"}),
    };
    let line = msg.to_json_line().expect("serialize");
    let parsed = AgentMessage::from_json_line(&line).expect("deserialize");
    match parsed {
        AgentMessage::Event { kind, data } => {
            assert_eq!(kind, "session_start");
            assert_eq!(data["session_id"], "abc-123");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn agent_message_error_roundtrip() {
    let msg = AgentMessage::Error { message: "connection refused".to_string() };
    let line = msg.to_json_line().expect("serialize");
    let parsed = AgentMessage::from_json_line(&line).expect("deserialize");
    match parsed {
        AgentMessage::Error { message } => assert_eq!(message, "connection refused"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn agent_message_all_variants_serialize_without_panic() {
    let variants: Vec<AgentMessage> = vec![
        AgentMessage::Ping,
        AgentMessage::Pong,
        AgentMessage::UserMessage { content: "test".to_string() },
        AgentMessage::AssistantMessage { content: "response".to_string() },
        AgentMessage::ToolCall { id: "id1".to_string(), name: "tool".to_string(), input: serde_json::json!({}) },
        AgentMessage::ToolResult { tool_use_id: "id1".to_string(), content: "result".to_string(), is_error: false },
        AgentMessage::Event { kind: "tick".to_string(), data: serde_json::json!(null) },
        AgentMessage::Error { message: "oops".to_string() },
    ];
    for variant in &variants {
        let line = variant.to_json_line().expect("all variants must serialize");
        assert!(line.ends_with('\n'), "must produce newline-terminated line");
    }
}

#[test]
fn unknown_message_type_gives_error() {
    let bad = r#"{"type":"unknown_type","foo":"bar"}"#;
    let result = AgentMessage::from_json_line(bad);
    assert!(result.is_err(), "unknown type tag must return Err");
}

#[test]
fn malformed_json_gives_error() {
    let bad = "not json at all {{{";
    let result = AgentMessage::from_json_line(bad);
    assert!(result.is_err(), "malformed JSON must return Err");
}

#[test]
fn empty_line_gives_error() {
    let result = AgentMessage::from_json_line("");
    assert!(result.is_err(), "empty input must return Err");
}

#[test]
fn whitespace_only_line_gives_error() {
    let result = AgentMessage::from_json_line("   \n  ");
    assert!(result.is_err(), "whitespace-only must return Err");
}

#[test]
fn json_line_has_trailing_newline() {
    let msg = AgentMessage::Ping;
    let line = msg.to_json_line().expect("serialize");
    assert!(line.ends_with('\n'));
    let trimmed = line.trim_end_matches('\n');
    serde_json::from_str::<serde_json::Value>(trimmed).expect("trimmed must be valid JSON");
}

// ---------------------------------------------------------------------------
// SyncMode
// ---------------------------------------------------------------------------

#[test]
fn sync_mode_default_is_manual() {
    let mode: SyncMode = Default::default();
    assert!(matches!(mode, SyncMode::Manual), "SyncMode default must be Manual");
}

#[test]
fn sync_mode_serializes_deserializes() {
    let manual = SyncMode::Manual;
    let json = serde_json::to_string(&manual).expect("serialize Manual");
    let back: SyncMode = serde_json::from_str(&json).expect("deserialize Manual");
    assert!(matches!(back, SyncMode::Manual));

    let auto_mode = SyncMode::Auto;
    let json = serde_json::to_string(&auto_mode).expect("serialize Auto");
    let back: SyncMode = serde_json::from_str(&json).expect("deserialize Auto");
    assert!(matches!(back, SyncMode::Auto));
}

// ---------------------------------------------------------------------------
// Config structs
// ---------------------------------------------------------------------------

#[test]
fn ssh_remote_config_defaults() {
    let config = SshRemoteConfig::default();
    assert_eq!(config.sync_mode, "manual");
    assert_eq!(config.ssh.port, 22);
    assert!(!config.ssh.agent_forwarding);
    assert!(config.ssh.host.is_empty());
    assert!(config.ssh.user.is_empty());
    assert!(config.ssh.key_file.is_none());
}

#[test]
fn ssh_config_defaults() {
    let config = SshConfig::default();
    assert_eq!(config.port, 22);
    assert!(!config.agent_forwarding);
    assert!(config.host.is_empty());
    assert!(config.user.is_empty());
    assert!(config.key_file.is_none());
}

#[test]
fn archon_config_has_remote_section() {
    let config = archon_core::config::ArchonConfig::default();
    let toml_str = toml::to_string(&config).expect("serialize ArchonConfig");
    assert!(
        toml_str.contains("sync_mode") || toml_str.contains("remote"),
        "ArchonConfig must serialize remote section"
    );
}

// ---------------------------------------------------------------------------
// HeadlessRuntime
// ---------------------------------------------------------------------------

#[test]
fn headless_runtime_new_stores_session_id() {
    let rt = HeadlessRuntime::new("test-session-42".to_string());
    assert_eq!(rt.session_id, "test-session-42");
}

#[test]
fn headless_runtime_new_with_empty_session_id() {
    let rt = HeadlessRuntime::new(String::new());
    assert_eq!(rt.session_id, "");
}

#[test]
fn headless_runtime_new_with_uuid_like_id() {
    let id = "550e8400-e29b-41d4-a716-446655440000".to_string();
    let rt = HeadlessRuntime::new(id.clone());
    assert_eq!(rt.session_id, id);
}

// ---------------------------------------------------------------------------
// Target parsing (user@host)
// ---------------------------------------------------------------------------

#[test]
fn parse_user_at_host() {
    let target = "user@hostname.example.com";
    let (user, host) = target.split_once('@').unwrap();
    assert_eq!(user, "user");
    assert_eq!(host, "hostname.example.com");
}

#[test]
fn parse_host_only_defaults_root() {
    let target = "hostname.example.com";
    let (user, host) = target.split_once('@').unwrap_or(("root", target));
    assert_eq!(user, "root");
    assert_eq!(host, "hostname.example.com");
}

#[test]
fn parse_user_at_ip_address() {
    let target = "admin@192.168.1.100";
    let (user, host) = target.split_once('@').unwrap();
    assert_eq!(user, "admin");
    assert_eq!(host, "192.168.1.100");
}

#[test]
fn parse_ip_only_defaults_root() {
    let target = "10.0.0.1";
    let (user, host) = target.split_once('@').unwrap_or(("root", target));
    assert_eq!(user, "root");
    assert_eq!(host, "10.0.0.1");
}
