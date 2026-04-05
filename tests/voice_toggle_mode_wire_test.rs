//! Integration test for TASK-WIRE-009: real archon binary logs the
//! configured voice.toggle_mode at startup, proving the flag is read.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SPAWN_TIMEOUT: Duration = Duration::from_secs(30);
const TOGGLE_LOG: &str = "voice: toggle_mode=true";
const PUSH_TO_TALK_LOG: &str = "voice: toggle_mode=false";

fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

fn minimal_config(toggle_mode: bool) -> String {
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
enabled = true
device = "default"
vad_threshold = 0.02
stt_provider = "mock"
stt_api_key = ""
stt_url = "http://localhost:9999"
hotkey = "ctrl+v"
toggle_mode = {toggle_mode}
"#
    )
}

fn run_and_scrape(toggle_mode: bool) -> String {
    let bin = archon_bin().expect("archon binary not built");
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("archon");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), minimal_config(toggle_mode)).unwrap();
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
fn voice_toggle_mode_true_logs_toggle_sentinel() {
    if archon_bin().is_none() {
        return;
    }
    let logs = run_and_scrape(true);
    assert!(
        logs.contains(TOGGLE_LOG),
        "expected log {TOGGLE_LOG:?}\n--- logs ---\n{logs}\n---"
    );
    assert!(
        !logs.contains(PUSH_TO_TALK_LOG),
        "should NOT contain {PUSH_TO_TALK_LOG:?}\n--- logs ---\n{logs}\n---"
    );
}

#[test]
fn voice_toggle_mode_false_logs_push_to_talk_sentinel() {
    if archon_bin().is_none() {
        return;
    }
    let logs = run_and_scrape(false);
    assert!(
        logs.contains(PUSH_TO_TALK_LOG),
        "expected log {PUSH_TO_TALK_LOG:?}\n--- logs ---\n{logs}\n---"
    );
    assert!(
        !logs.contains(TOGGLE_LOG),
        "should NOT contain {TOGGLE_LOG:?}\n--- logs ---\n{logs}\n---"
    );
}
