//! Integration test for TASK-WIRE-007: voice loop wired into archon binary.
//!
//! Spawns archon with config.voice.enabled=true and config.voice.enabled=false,
//! scrapes the log file for the wiring/disabled sentinel, and asserts the
//! voice pipeline was actually constructed + spawned when enabled.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WIRED_LOG: &str = "voice: pipeline wired";
const DISABLED_LOG: &str = "voice: disabled";
const STARTED_LOG: &str = "voice: pipeline started";
const SPAWN_TIMEOUT: Duration = Duration::from_secs(30);

fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

fn minimal_config(voice_enabled: bool) -> String {
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

[voice]
enabled = {voice_enabled}
device = "default"
vad_threshold = 0.02
stt_provider = "mock"
stt_api_key = ""
stt_url = "http://localhost:9999"
hotkey = "ctrl+v"
toggle_mode = true
"#
    )
}

fn run_and_scrape(voice_enabled: bool) -> String {
    let bin = archon_bin().expect("archon binary not built");
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("archon");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), minimal_config(voice_enabled)).unwrap();
    let log_dir = tmp.path().join("data").join("archon").join("logs");
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).unwrap();

    let mut child = Command::new(&bin)
        .current_dir(&work_dir)
        .env("ARCHON_CONFIG_DIR", &config_dir)
        .env("ANTHROPIC_API_KEY", "sk-fake-test-key-not-real")
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_CONFIG_HOME", tmp.path())
        .env("RUST_LOG", "info")
        .arg("-p")
        .arg("hello")
        .arg("--output-format")
        .arg("text")
        .arg("--no-session-persistence")
        .arg("--max-turns")
        .arg("1")
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
fn voice_enabled_logs_pipeline_wired_and_started() {
    if archon_bin().is_none() {
        return;
    }
    let logs = run_and_scrape(true);
    assert!(
        logs.contains(WIRED_LOG),
        "expected log {WIRED_LOG:?} when voice enabled\n--- logs ---\n{logs}\n---"
    );
    assert!(
        logs.contains(STARTED_LOG),
        "expected voice_loop to log {STARTED_LOG:?} (proves task was spawned & running)\n--- logs ---\n{logs}\n---"
    );
    assert!(
        !logs.contains(DISABLED_LOG),
        "should NOT contain disabled log when voice.enabled=true\n{logs}"
    );
}

#[test]
fn voice_disabled_logs_disabled_sentinel() {
    if archon_bin().is_none() {
        return;
    }
    let logs = run_and_scrape(false);
    assert!(
        logs.contains(DISABLED_LOG),
        "expected log {DISABLED_LOG:?} when voice disabled\n--- logs ---\n{logs}\n---"
    );
    assert!(
        !logs.contains(WIRED_LOG),
        "should NOT contain wired log when voice.enabled=false\n{logs}"
    );
    assert!(
        !logs.contains(STARTED_LOG),
        "should NOT contain pipeline-started log when disabled\n{logs}"
    );
}
