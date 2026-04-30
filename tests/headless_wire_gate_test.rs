//! Integration tests for GHOST-002: headless mode JSON-lines I/O loop.
//!
//! Covers:
//! - AgentMessage protocol round-trip serialization
//! - Headless Ping → Pong over stdin/stdout
//! - Headless invalid JSON → Error response
//! - Headless UserMessage → agent processing (smoke test — logs for wiring)

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Protocol unit tests (no binary needed)
// ---------------------------------------------------------------------------

#[test]
fn agent_message_user_message_round_trip() {
    let msg = archon_core::remote::protocol::AgentMessage::UserMessage {
        content: "hello world".to_string(),
    };
    let line = msg.to_json_line().unwrap();
    let parsed = archon_core::remote::protocol::AgentMessage::from_json_line(&line).unwrap();
    match parsed {
        archon_core::remote::protocol::AgentMessage::UserMessage { content } => {
            assert_eq!(content, "hello world");
        }
        _ => panic!("expected UserMessage, got {parsed:?}"),
    }
}

#[test]
fn agent_message_ping_pong_round_trip() {
    let ping = archon_core::remote::protocol::AgentMessage::Ping;
    let line = ping.to_json_line().unwrap();
    assert!(line.contains("\"ping\""), "line should contain ping: {line}");

    let parsed = archon_core::remote::protocol::AgentMessage::from_json_line(&line).unwrap();
    assert!(matches!(parsed, archon_core::remote::protocol::AgentMessage::Ping));

    let pong = archon_core::remote::protocol::AgentMessage::Pong;
    let line = pong.to_json_line().unwrap();
    assert!(line.contains("\"pong\""), "line should contain pong: {line}");

    let parsed = archon_core::remote::protocol::AgentMessage::from_json_line(&line).unwrap();
    assert!(matches!(parsed, archon_core::remote::protocol::AgentMessage::Pong));
}

#[test]
fn agent_message_error_round_trip() {
    let err = archon_core::remote::protocol::AgentMessage::Error {
        message: "something went wrong".to_string(),
    };
    let line = err.to_json_line().unwrap();
    let parsed = archon_core::remote::protocol::AgentMessage::from_json_line(&line).unwrap();
    match parsed {
        archon_core::remote::protocol::AgentMessage::Error { message } => {
            assert_eq!(message, "something went wrong");
        }
        _ => panic!("expected Error"),
    }
}

#[test]
fn agent_message_assistant_message_round_trip() {
    let msg = archon_core::remote::protocol::AgentMessage::AssistantMessage {
        content: "I think therefore I am".to_string(),
    };
    let line = msg.to_json_line().unwrap();
    let parsed = archon_core::remote::protocol::AgentMessage::from_json_line(&line).unwrap();
    match parsed {
        archon_core::remote::protocol::AgentMessage::AssistantMessage { content } => {
            assert_eq!(content, "I think therefore I am");
        }
        _ => panic!("expected AssistantMessage, got {parsed:?}"),
    }
}

#[test]
fn agent_message_empty_input_rejected() {
    let result = archon_core::remote::protocol::AgentMessage::from_json_line("");
    assert!(result.is_err());
    let result = archon_core::remote::protocol::AgentMessage::from_json_line("   \n  ");
    assert!(result.is_err());
}

#[test]
fn agent_message_invalid_json_returns_error() {
    let result = archon_core::remote::protocol::AgentMessage::from_json_line("not json at all");
    assert!(result.is_err());
    let result = archon_core::remote::protocol::AgentMessage::from_json_line(r#"{"type":"unknown_variant","x":1}"#);
    assert!(result.is_err());
}

#[test]
fn agent_message_event_with_data() {
    let event = archon_core::remote::protocol::AgentMessage::Event {
        kind: "session_start".to_string(),
        data: serde_json::json!({"session_id": "abc-123"}),
    };
    let line = event.to_json_line().unwrap();
    let parsed = archon_core::remote::protocol::AgentMessage::from_json_line(&line).unwrap();
    match parsed {
        archon_core::remote::protocol::AgentMessage::Event { kind, data } => {
            assert_eq!(kind, "session_start");
            assert_eq!(data["session_id"], "abc-123");
        }
        _ => panic!("expected Event, got {parsed:?}"),
    }
}

#[test]
fn agent_message_tool_call_round_trip() {
    let tc = archon_core::remote::protocol::AgentMessage::ToolCall {
        id: "tc_001".to_string(),
        name: "Bash".to_string(),
        input: serde_json::json!({"command": "ls"}),
    };
    let line = tc.to_json_line().unwrap();
    let parsed = archon_core::remote::protocol::AgentMessage::from_json_line(&line).unwrap();
    match parsed {
        archon_core::remote::protocol::AgentMessage::ToolCall { id, name, input } => {
            assert_eq!(id, "tc_001");
            assert_eq!(name, "Bash");
            assert_eq!(input["command"], "ls");
        }
        _ => panic!("expected ToolCall, got {parsed:?}"),
    }
}

#[test]
fn agent_message_tool_result_round_trip() {
    let tr = archon_core::remote::protocol::AgentMessage::ToolResult {
        tool_use_id: "tc_001".to_string(),
        content: "file1 file2".to_string(),
        is_error: false,
    };
    let line = tr.to_json_line().unwrap();
    let parsed = archon_core::remote::protocol::AgentMessage::from_json_line(&line).unwrap();
    match parsed {
        archon_core::remote::protocol::AgentMessage::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            assert_eq!(tool_use_id, "tc_001");
            assert_eq!(content, "file1 file2");
            assert!(!is_error);
        }
        _ => panic!("expected ToolResult, got {parsed:?}"),
    }
}

// ---------------------------------------------------------------------------
// Integration tests (require archon binary)
// ---------------------------------------------------------------------------

fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

fn minimal_config() -> String {
    r#"
[api]
default_model = "claude-sonnet-4-6"
thinking_budget = 16384
default_effort = "high"
max_retries = 3

[identity]
mode = "clean"
anti_distillation = false

[personality]
name = "Archon"
type = "INTJ"
enneagram = "4w5"
traits = ["strategic", "direct"]
communication_style = "terse"

[consciousness]
inner_voice = false
energy_decay_rate = 0.02
initial_rules = []

[tools]
bash_timeout = 120
bash_max_output = 102400
max_concurrency = 4

[permissions]
mode = "bypassPermissions"
allow_paths = []
deny_paths = []

[tui]
vim_mode = false

[context]
compact_threshold = 0.8
preserve_recent_turns = 3
prompt_cache = false

[memory]
enabled = false

[cost]
warn_threshold = 100.0
hard_limit = 0.0

[logging]
level = "info"
max_files = 50
max_file_size_mb = 10

[session]
auto_resume = false

[checkpoint]
enabled = false
max_checkpoints = 10
"#
    .to_string()
}

/// Spawn headless archon, send a Ping, and verify Pong comes back.
#[test]
fn headless_ping_pong_round_trip() {
    let bin = match archon_bin() {
        Some(b) => b,
        None => return, // skip if binary not built
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("archon");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), minimal_config()).unwrap();
    let log_dir = tmp.path().join("data").join("archon").join("logs");
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).unwrap();

    let mut child = Command::new(&bin)
        .current_dir(&work_dir)
        .env("ARCHON_CONFIG_DIR", &config_dir)
        .env("ANTHROPIC_API_KEY", "sk-fake-test-key-not-real")
        .env("ARCHON_LOG_DIR", &log_dir)
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_CONFIG_HOME", tmp.path())
        .arg("--headless")
        .arg("--session-id")
        .arg("test-headless-ping")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn archon");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());

    // Send Ping
    let ping = serde_json::json!({"type": "ping"});
    writeln!(stdin, "{ping}").unwrap();
    stdin.flush().unwrap();

    // Wait for Pong
    let start = Instant::now();
    let mut response = String::new();
    loop {
        if start.elapsed() > Duration::from_secs(10) {
            break;
        }
        use std::io::BufRead;
        response.clear();
        match stdout.read_line(&mut response) {
            Ok(0) => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Ok(_) => break,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();

    assert!(
        response.contains("\"pong\""),
        "expected Pong response, got: {response}"
    );
}

/// Spawn headless archon, send invalid JSON, and verify Error comes back.
#[test]
fn headless_invalid_json_returns_error() {
    let bin = match archon_bin() {
        Some(b) => b,
        None => return,
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("archon");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), minimal_config()).unwrap();
    let log_dir = tmp.path().join("data").join("archon").join("logs");
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).unwrap();

    let mut child = Command::new(&bin)
        .current_dir(&work_dir)
        .env("ARCHON_CONFIG_DIR", &config_dir)
        .env("ANTHROPIC_API_KEY", "sk-fake-test-key-not-real")
        .env("ARCHON_LOG_DIR", &log_dir)
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_CONFIG_HOME", tmp.path())
        .arg("--headless")
        .arg("--session-id")
        .arg("test-headless-err")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn archon");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());

    // Send invalid JSON
    writeln!(stdin, "this is not json at all").unwrap();
    stdin.flush().unwrap();

    let start = Instant::now();
    let mut response = String::new();
    loop {
        if start.elapsed() > Duration::from_secs(10) {
            break;
        }
        use std::io::BufRead;
        response.clear();
        match stdout.read_line(&mut response) {
            Ok(0) => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Ok(_) => break,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();

    assert!(
        response.contains("\"error\""),
        "expected Error response for invalid JSON, got: {response}"
    );
}
