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
/// Blocking on a worker thread keeps the read honest. What it was bounded with
/// -- a fixed budget -- could not tell "the protocol is broken" from "this
/// runner is busy", so the budget had to be sized for the worst runner and
/// still guessed wrong: 10s, then 30s, then a red main when a Windows runner
/// needed longer than 30s for a handshake that takes ~2s on an idle machine.
///
/// `try_wait` answers the question the budget was standing in for. A child that
/// is still running is making progress and is worth waiting for; a child that
/// has exited is a failure worth reporting *immediately*, with its real exit
/// status, instead of after a 30s stare at a process that is already dead.
fn read_line_while_alive(
    stdout: std::process::ChildStdout,
    child: &mut std::process::Child,
    ceiling: Duration,
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

    let start = std::time::Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(outcome) => return outcome,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("reader thread stopped without an answer".to_string());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("child exited ({status}) before writing a line"));
        }
        if start.elapsed() >= ceiling {
            return Err(format!("child alive but silent for {ceiling:?}"));
        }
    }
}

/// Hang backstop for the spawned binary.
///
/// Deliberately not a performance budget. A live child is waited for however
/// long it needs, so this only fires when headless has genuinely wedged -- and
/// a wedge is worth two minutes of CI to catch, because nothing else would.
const HEADLESS_HANG_CEILING: Duration = Duration::from_secs(120);

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

/// A spawned headless archon, plus everything needed to explain a silent one.
struct Headless {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: Option<std::process::ChildStdout>,
    stderr: std::process::ChildStderr,
    log_dir: PathBuf,
    /// Held for as long as the child runs. Dropping it removes the config,
    /// data and log directories out from under a live process.
    _tmp: tempfile::TempDir,
}

/// Spawn headless archon against throwaway config, data and log directories.
///
/// Returns `None` when the binary was not built, which is the pre-existing
/// skip condition for these two tests.
fn spawn_headless(session_id: &str) -> Option<Headless> {
    let bin = archon_bin()?;

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
        .arg(session_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn archon");

    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    Some(Headless {
        child,
        stdin,
        stdout: Some(stdout),
        stderr,
        log_dir,
        _tmp: tmp,
    })
}

impl Drop for Headless {
    /// Kill the child before the temp directory goes away.
    ///
    /// That directory holds the child's config, database and logs. Removing it
    /// under a live process leaves a headless archon running against files that
    /// no longer exist -- and on Windows the child's open handles make the
    /// removal fail silently instead, so the runner accumulates both a stray
    /// process and its litter. Running before the fields drop is exactly what
    /// an explicit `Drop` on the owner buys.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Headless {
    /// Send one line and return the child's first reply, or panic describing why
    /// none came.
    fn round_trip(&mut self, request: &str, expected: &str) -> String {
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();

        let stdout = self.stdout.take().expect("stdout taken once");
        match read_line_while_alive(stdout, &mut self.child, HEADLESS_HANG_CEILING) {
            Ok(line) => line,
            Err(reason) => panic!("no {expected}: {reason}{}", self.diagnostics()),
        }
    }

    /// Exit status, stderr and the child's own log files.
    ///
    /// The old message read `status` *after* `child.kill()`. On Windows that is
    /// `TerminateProcess(handle, 1)`, so a healthy-but-slow child was reported
    /// as "exited 1" -- pointing every reader at a crash that never happened.
    /// Status is read before the kill now, and the log directory the test
    /// already configures is included, because headless writes its startup
    /// trace to a file rather than to stderr: without it a failure says only
    /// `stderr=`, which is exactly as much as no diagnostics at all.
    fn diagnostics(&mut self) -> String {
        let status = match self.child.try_wait() {
            Ok(Some(status)) => format!("exited {status}"),
            Ok(None) => "still running".to_string(),
            Err(error) => format!("unknown ({error})"),
        };
        let _ = self.child.kill();
        let _ = self.child.wait();

        let mut stderr_text = String::new();
        let _ = self.stderr.read_to_string(&mut stderr_text);

        let mut logs = String::new();
        if let Ok(entries) = std::fs::read_dir(&self.log_dir) {
            for entry in entries.flatten() {
                if let Ok(contents) = std::fs::read_to_string(entry.path()) {
                    logs.push_str(&contents);
                }
            }
        }
        if logs.is_empty() {
            logs.push_str("(no log files written)");
        }

        format!("; child={status}; stderr={stderr_text}; logs={logs}")
    }
}

/// Spawn headless archon, send a Ping, and verify Pong comes back.
#[test]
fn headless_ping_pong_round_trip() {
    let Some(mut headless) = spawn_headless("test-headless-ping") else {
        return; // skip if binary not built
    };
    let ping = serde_json::json!({"type": "ping"});
    let response = headless.round_trip(&ping.to_string(), "Pong");
    assert!(
        response.contains("\"pong\""),
        "expected Pong response, got: {response}{}",
        headless.diagnostics()
    );
}

/// Spawn headless archon, send invalid JSON, and verify Error comes back.
#[test]
fn headless_invalid_json_returns_error() {
    let Some(mut headless) = spawn_headless("test-headless-err") else {
        return;
    };
    let response = headless.round_trip("this is not json at all", "Error reply");
    assert!(
        response.contains("\"error\""),
        "expected Error response for invalid JSON, got: {response}{}",
        headless.diagnostics()
    );
}
