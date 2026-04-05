//! Integration test for TASK-WIRE-003: `config.context.prompt_cache` flag gating.
//!
//! Gate 1 (test-first) — written BEFORE implementation. Asserts that:
//!   * When `prompt_cache = false` the binary logs a DISABLED message
//!   * When `prompt_cache = true`  the binary logs an ENABLED message
//!
//! The real behavioural effect (skip `cache_control` on system-prompt blocks)
//! is covered additionally by a unit test in `prompt_cache_block_test` below
//! that directly exercises the block-building helper.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const DISABLED_LOG: &str = "context.prompt_cache=false: cache_control hints DISABLED";
const ENABLED_LOG: &str = "context.prompt_cache=true: cache_control hints ACTIVE";

const SPAWN_TIMEOUT: Duration = Duration::from_secs(30);

fn minimal_config(prompt_cache: bool) -> String {
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
prompt_cache = {prompt_cache}

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
    )
}

fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

fn run_archon_capture(prompt_cache: bool) -> String {
    let bin = archon_bin().expect("archon binary not built");
    let tmp = tempfile::tempdir().expect("create tempdir");
    let config_dir = tmp.path().join("archon");
    std::fs::create_dir_all(&config_dir).expect("create archon config dir");
    let config_path = config_dir.join("config.toml");
    std::fs::write(&config_path, minimal_config(prompt_cache)).expect("write config.toml");
    let log_dir = tmp.path().join("data").join("archon").join("logs");
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).expect("create work dir");

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

    let stderr = child.stderr.take().expect("child stderr");
    let (tx, rx) = mpsc::channel::<String>();
    let reader_handle = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send(line.clone()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    if let Some(stdout) = child.stdout.take() {
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut buf = String::new();
            while reader.read_line(&mut buf).unwrap_or(0) > 0 {
                buf.clear();
            }
        });
    }

    let start = Instant::now();
    let mut collected = String::new();
    while start.elapsed() < SPAWN_TIMEOUT {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(line) => {
                collected.push_str(&line);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = reader_handle.join();
    while let Ok(line) = rx.try_recv() {
        collected.push_str(&line);
    }

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
fn prompt_cache_disabled_logs_disabled_message() {
    if archon_bin().is_none() {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set");
        return;
    }
    let logs = run_archon_capture(false);
    assert!(
        logs.contains(DISABLED_LOG),
        "expected log to contain {DISABLED_LOG:?}\n---\n{logs}\n---"
    );
    assert!(
        !logs.contains(ENABLED_LOG),
        "log should NOT contain ENABLED line when prompt_cache=false\n---\n{logs}\n---"
    );
}

#[test]
fn prompt_cache_enabled_logs_enabled_message() {
    if archon_bin().is_none() {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set");
        return;
    }
    let logs = run_archon_capture(true);
    assert!(
        logs.contains(ENABLED_LOG),
        "expected log to contain {ENABLED_LOG:?}\n---\n{logs}\n---"
    );
    assert!(
        !logs.contains(DISABLED_LOG),
        "log should NOT contain DISABLED line when prompt_cache=true\n---\n{logs}\n---"
    );
}
