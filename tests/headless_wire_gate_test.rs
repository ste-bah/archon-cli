//! Integration tests for GHOST-002: headless mode JSON-lines I/O loop.
//!
//! Covers:
//! - AgentMessage protocol round-trip serialization
//! - Headless Ping → Pong over stdin/stdout
//! - Headless invalid JSON → Error response
//! - Headless UserMessage → agent processing (smoke test — logs for wiring)

use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

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
    assert!(
        line.contains("\"ping\""),
        "line should contain ping: {line}"
    );

    let parsed = archon_core::remote::protocol::AgentMessage::from_json_line(&line).unwrap();
    assert!(matches!(
        parsed,
        archon_core::remote::protocol::AgentMessage::Ping
    ));

    let pong = archon_core::remote::protocol::AgentMessage::Pong;
    let line = pong.to_json_line().unwrap();
    assert!(
        line.contains("\"pong\""),
        "line should contain pong: {line}"
    );

    let parsed = archon_core::remote::protocol::AgentMessage::from_json_line(&line).unwrap();
    assert!(matches!(
        parsed,
        archon_core::remote::protocol::AgentMessage::Pong
    ));
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
    let result = archon_core::remote::protocol::AgentMessage::from_json_line(
        r#"{"type":"unknown_variant","x":1}"#,
    );
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

/// Read one line from the child's stdout, or give up after `budget`.
///
/// `BufReader::read_line` blocks until it has a line or hits EOF, so polling it
/// in a retry loop cannot work. The previous version did exactly that and
/// treated `Ok(0)` as "nothing yet, sleep and try again" — but `Ok(0)` *is*
/// EOF. Once the child exited, every call returned instantly, the loop spun for
/// the entire timeout, and the test then failed on an empty response. That is
/// the shape of the 10.2s macOS CI failure: not a slow pong, a dead child that
/// took ten seconds to be reported as one.
///
/// Blocking on a worker thread and bounding it with `recv_timeout` keeps the
/// timeout meaningful and distinguishes the two causes, so a failure says which
/// happened instead of leaving it to be guessed.
fn read_line_with_timeout(
    stdout: std::process::ChildStdout,
    budget: Duration,
) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        let outcome = match reader.read_line(&mut line) {
            Ok(0) => Err("child closed stdout without writing a line (EOF)".to_string()),
            Ok(_) => Ok(line),
            Err(error) => Err(format!("read error: {error}")),
        };
        let _ = tx.send(outcome);
    });
    rx.recv_timeout(budget)
        .unwrap_or_else(|_| Err(format!("no line within {budget:?}")))
}

/// Cold-start budget for the spawned binary.
///
/// The old 10s was measured against a warm local machine. A loaded CI runner
/// starting a fresh process, reading config and initialising its runtime can
/// legitimately need longer, and an under-budgeted wait is indistinguishable
/// from a real protocol break.
const HEADLESS_REPLY_BUDGET: Duration = Duration::from_secs(30);

fn minimal_config() -> String {
    r#"
[api]
default_model = "claude-sonnet-4-6"
thinking_budget = 16384
default_effort = "high"
max_retries = 3

[llm]
provider = "local"

[llm.local]
base_url = "http://127.0.0.1:9/v1"
model = "test-local"
timeout_secs = 1
pull_if_missing = false

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
        // XDG_DATA_HOME alone does not redirect the child's memory database:
        // `dirs::data_dir()` reads it only on Linux, and falls back to the
        // shell known-folder API on Windows and ~/Library/Application Support
        // on macOS. Without this the spawned binary opened the runner's real
        // memory.db, which every other concurrent test is also using, and Cozo
        // panicked on open with `database is locked` -- so the child exited 1
        // having written nothing and the test reported a bare stdout EOF.
        .env("ARCHON_DATA_DIR", tmp.path().join("data").join("archon"))
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
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    // Send Ping
    let ping = serde_json::json!({"type": "ping"});
    writeln!(stdin, "{ping}").unwrap();
    stdin.flush().unwrap();

    let result = read_line_with_timeout(stdout, HEADLESS_REPLY_BUDGET);

    let _ = child.kill();
    let status = child.wait().ok();
    let mut stderr_text = String::new();
    let _ = stderr.read_to_string(&mut stderr_text);

    let response = match result {
        Ok(line) => line,
        Err(reason) => panic!("no Pong: {reason}; status={status:?}; stderr={stderr_text}"),
    };

    assert!(
        response.contains("\"pong\""),
        "expected Pong response, got: {response}; status={status:?}; stderr={stderr_text}"
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
        // XDG_DATA_HOME alone does not redirect the child's memory database:
        // `dirs::data_dir()` reads it only on Linux, and falls back to the
        // shell known-folder API on Windows and ~/Library/Application Support
        // on macOS. Without this the spawned binary opened the runner's real
        // memory.db, which every other concurrent test is also using, and Cozo
        // panicked on open with `database is locked` -- so the child exited 1
        // having written nothing and the test reported a bare stdout EOF.
        .env("ARCHON_DATA_DIR", tmp.path().join("data").join("archon"))
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
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    // Send invalid JSON
    writeln!(stdin, "this is not json at all").unwrap();
    stdin.flush().unwrap();

    let result = read_line_with_timeout(stdout, HEADLESS_REPLY_BUDGET);

    let _ = child.kill();
    let status = child.wait().ok();
    let mut stderr_text = String::new();
    let _ = stderr.read_to_string(&mut stderr_text);

    let response = match result {
        Ok(line) => line,
        Err(reason) => panic!("no Error reply: {reason}; status={status:?}; stderr={stderr_text}"),
    };

    assert!(
        response.contains("\"error\""),
        "expected Error response for invalid JSON, got: {response}; status={status:?}; stderr={stderr_text}"
    );
}
