//! Noninteractive startup must not wire voice, regardless of toggle mode.
//! Interactive ordering and receiver delivery are covered by main_startup_tests.

use std::io::Read;
use std::process::{Command, Stdio};

const TOGGLE_LOG: &str = "voice: toggle_mode=true";
const PUSH_TO_TALK_LOG: &str = "voice: toggle_mode=false";

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
device = "nonexistent-voice-test-device"
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
    let bin = env!("CARGO_BIN_EXE_archon");
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("archon");
    std::fs::create_dir_all(&config_dir).unwrap();
    let config = minimal_config(toggle_mode);
    let parsed: archon_core::config::ArchonConfig = toml::from_str(&config).expect("valid config");
    assert!(parsed.voice.enabled);
    assert_eq!(parsed.voice.toggle_mode, toggle_mode);
    std::fs::write(config_dir.join("config.toml"), config).unwrap();
    let log_dir = tmp.path().join("data").join("archon").join("logs");
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).unwrap();

    let status = Command::new(bin)
        .current_dir(&work_dir)
        .env("ARCHON_CONFIG_DIR", &config_dir)
        .env("ANTHROPIC_API_KEY", "sk-fake-test-key-not-real")
        .env("ARCHON_LOG_DIR", &log_dir)
        // XDG_DATA_HOME only redirects `dirs::data_dir()` on Linux -- Windows
        // reads the shell known-folder API and macOS ~/Library/Application
        // Support, so without this the child opens the real user database.
        .env("ARCHON_DATA_DIR", tmp.path().join("data").join("archon"))
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_CONFIG_HOME", tmp.path())
        .env("ARCHON_CACHE_ROOT", tmp.path().join("cache"))
        .env("ARCHON_TMPDIR", tmp.path().join("scratch"))
        .env("RUST_LOG", "info")
        .arg("--setting-sources")
        .arg("user")
        .arg("--list-themes")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run archon catalog mode");
    assert!(status.success(), "catalog command failed: {status}");

    let entries: Vec<_> = std::fs::read_dir(&log_dir)
        .expect("startup log directory")
        .map(|entry| entry.expect("startup log entry"))
        .collect();
    assert_eq!(entries.len(), 1, "expected one startup log");
    let mut collected = String::new();
    std::fs::File::open(entries[0].path())
        .expect("open startup log")
        .take(64 * 1024 + 1)
        .read_to_string(&mut collected)
        .expect("read startup log");
    assert!(
        collected.len() <= 64 * 1024,
        "startup log exceeded test bound"
    );
    assert!(!collected.is_empty(), "startup log is empty");
    collected
}

#[test]
fn voice_toggle_mode_true_does_not_wire_for_catalog_mode() {
    assert_catalog_skips_voice(true);
}

#[test]
fn voice_toggle_mode_false_does_not_wire_for_catalog_mode() {
    assert_catalog_skips_voice(false);
}

fn assert_catalog_skips_voice(toggle_mode: bool) {
    let logs = run_and_scrape(toggle_mode);
    assert!(
        !logs.contains(TOGGLE_LOG) && !logs.contains(PUSH_TO_TALK_LOG),
        "catalog mode wired voice with toggle_mode={toggle_mode}"
    );
    assert!(!logs.contains("voice: capture device"));
    assert!(!logs.contains("voice: pipeline"));
    assert!(!logs.contains("voice: unavailable"));
}
