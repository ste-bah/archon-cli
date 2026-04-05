//! Integration test for TASK-WIRE-008: config.ssh.agent_forwarding is
//! threaded into the SshConnectionConfig built by the `archon remote ssh`
//! handler.
//!
//! Runs the real archon binary twice (forwarding=true/false), scrapes the
//! log file for the sentinel log that echoes the wired value, and asserts
//! both branches flow through.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const FWD_TRUE_LOG: &str = "remote ssh: agent_forwarding=true";
const FWD_FALSE_LOG: &str = "remote ssh: agent_forwarding=false";
const SPAWN_TIMEOUT: Duration = Duration::from_secs(20);

fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

fn minimal_config(agent_forwarding: bool) -> String {
    format!(
        r#"
[api]
default_model = "claude-sonnet-4-6"
thinking_budget = 16384
default_effort = "high"
max_retries = 3

[identity]
mode = "spoof"
spoof_version = "2.1.89"
spoof_entrypoint = "cli"
anti_distillation = false

[personality]
name = "Archon"
type = "INTJ"
enneagram = "4w5"
traits = ["strategic"]
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

[voice]
enabled = false
device = "default"
vad_threshold = 0.02
stt_provider = "mock"
stt_api_key = ""
stt_url = "http://localhost:9999"
hotkey = "ctrl+v"
toggle_mode = true

[remote.ssh]
agent_forwarding = {agent_forwarding}
"#
    )
}

fn run_and_scrape(agent_forwarding: bool) -> String {
    let bin = archon_bin().expect("archon binary not built");
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("archon");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        minimal_config(agent_forwarding),
    )
    .unwrap();
    let log_dir = tmp.path().join("data").join("archon").join("logs");
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).unwrap();

    // Use port 1 (always closed) — TCP connect will fail fast, but NOT before
    // the handler logs the built SshConnectionConfig.
    let mut child = Command::new(&bin)
        .current_dir(&work_dir)
        .env("ARCHON_CONFIG_DIR", &config_dir)
        .env("ANTHROPIC_API_KEY", "sk-fake-test-key-not-real")
        .env("ARCHON_LOG_DIR", &log_dir)
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_CONFIG_HOME", tmp.path())
        .env("RUST_LOG", "info")
        .arg("remote")
        .arg("ssh")
        .arg("--port")
        .arg("1")
        .arg("test@127.0.0.1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn archon");

    let start = Instant::now();
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        if start.elapsed() > SPAWN_TIMEOUT {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.wait();

    let mut collected = String::new();
    if let Ok(entries) = std::fs::read_dir(&log_dir) {
        for entry in entries.flatten() {
            if let Ok(contents) = std::fs::read_to_string(entry.path()) {
                collected.push_str(&contents);
            }
        }
    }
    collected
}

#[test]
fn ssh_agent_forwarding_true_is_threaded_through() {
    if archon_bin().is_none() {
        return;
    }
    let logs = run_and_scrape(true);
    assert!(
        logs.contains(FWD_TRUE_LOG),
        "expected log {FWD_TRUE_LOG:?} when config.ssh.agent_forwarding=true\n--- logs ---\n{logs}\n---"
    );
    assert!(
        !logs.contains(FWD_FALSE_LOG),
        "should NOT contain {FWD_FALSE_LOG:?} when config.ssh.agent_forwarding=true\n--- logs ---\n{logs}\n---"
    );
}

#[test]
fn ssh_agent_forwarding_false_is_threaded_through() {
    if archon_bin().is_none() {
        return;
    }
    let logs = run_and_scrape(false);
    assert!(
        logs.contains(FWD_FALSE_LOG),
        "expected log {FWD_FALSE_LOG:?} when config.ssh.agent_forwarding=false\n--- logs ---\n{logs}\n---"
    );
    assert!(
        !logs.contains(FWD_TRUE_LOG),
        "should NOT contain {FWD_TRUE_LOG:?} when config.ssh.agent_forwarding=false\n--- logs ---\n{logs}\n---"
    );
}
