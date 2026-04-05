//! Integration test for TASK-WIRE-002: `config.memory.enabled` flag gating.
//!
//! Gate 1 (test-first) — this test is written BEFORE the implementation and
//! is EXPECTED TO FAIL until `main.rs` is updated to:
//!
//!   * Skip registering `MemoryStoreTool` / `MemoryRecallTool` when
//!     `config.memory.enabled = false`
//!   * Skip calling `agent.set_memory_graph(...)` when disabled
//!   * Emit one of two exact log lines (see `DISABLED_LOG` / `ENABLED_LOG`)
//!
//! The test spawns the real `archon` binary twice with different temp
//! `config.toml` files and greps stderr (tracing output) for the log lines.
//! It uses `-p` (print mode) with a fake API key so the process reaches the
//! memory-wiring stage then exits quickly when the API call fails.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const DISABLED_LOG: &str = "memory.enabled=false: memory tools and graph injection DISABLED";
const ENABLED_LOG: &str = "memory.enabled=true: memory tools + graph injection ACTIVE";

/// Hard ceiling on how long we'll wait for the log lines to appear on stderr
/// before giving up and killing the child process.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(30);

/// Minimal config.toml with only the fields required by the loader. We keep
/// this tightly scoped so the test is insulated from unrelated config churn.
fn minimal_config(memory_enabled: bool) -> String {
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

[memory]
enabled = {memory_enabled}

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

/// Locate the archon binary that cargo built for this integration test.
///
/// Cargo sets `CARGO_BIN_EXE_archon` for integration tests when the package
/// defines a `[[bin]] name = "archon"`. If it is missing the binary hasn't
/// been built yet and we skip rather than fail — this keeps `cargo test`
/// ergonomic in fresh clones.
fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

/// Spawn archon with a temp config, wait for the child to exit (or timeout),
/// then read archon's log file from the temp XDG_DATA_HOME and return its
/// contents. Archon writes tracing output ONLY to a file (not stderr) to avoid
/// corrupting the TUI, so we scrape the log file instead of stderr.
fn run_archon_capture_stderr(memory_enabled: bool) -> String {
    let bin = archon_bin().expect("archon binary not built — run `cargo build` first");

    let tmp = tempfile::tempdir().expect("create tempdir");
    // archon reads its config from $XDG_CONFIG_HOME/archon/config.toml via
    // dirs::config_dir(). We set XDG_CONFIG_HOME to tmp.path() below, so
    // write the config into the nested archon/ subdir.
    let config_dir = tmp.path().join("archon");
    std::fs::create_dir_all(&config_dir).expect("create archon config dir");
    let config_path = config_dir.join("config.toml");
    std::fs::write(&config_path, minimal_config(memory_enabled)).expect("write config.toml");
    let log_dir = tmp.path().join("data").join("archon").join("logs");

    // Point archon at our temp config dir via ARCHON_CONFIG_DIR. We also
    // supply a fake ANTHROPIC_API_KEY so auth resolution succeeds and the
    // process reaches the memory-wiring stage. It will then fail when the
    // real API call is attempted — that's fine, we only care about the
    // startup log lines.
    // Isolate cwd so the binary doesn't pick up the real project's
    // {work_dir}/.archon/config.toml layer and override our test settings.
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).expect("create work dir");
    let mut child = Command::new(&bin)
        .current_dir(&work_dir)
        .env("ARCHON_CONFIG_DIR", &config_dir)
        .env("ANTHROPIC_API_KEY", "sk-fake-test-key-not-real")
        // Isolate from the user's real archon data/cache dirs.
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

    // Drain stderr on a worker thread into a channel so we can apply a
    // wall-clock timeout from the parent thread.
    let stderr = child.stderr.take().expect("child stderr");
    let (tx, rx) = mpsc::channel::<String>();
    let reader_handle = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    if tx.send(line.clone()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Also drain stdout so the child doesn't block on a full pipe.
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
    let mut saw_disabled = false;
    let mut saw_enabled = false;

    while start.elapsed() < SPAWN_TIMEOUT {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(line) => {
                if line.contains(DISABLED_LOG) {
                    saw_disabled = true;
                }
                if line.contains(ENABLED_LOG) {
                    saw_enabled = true;
                }
                collected.push_str(&line);
                // Exit early: we only need to see one of them to know the
                // wiring was hit. If we saw the one we were looking for,
                // bail out immediately.
                if (memory_enabled && saw_enabled) || (!memory_enabled && saw_disabled) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Check whether the child has already exited.
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Tear down the child. Best effort — the process may have already
    // exited on its own when the fake API key was rejected.
    let _ = child.kill();
    let _ = child.wait();
    // Close stdin channel by dropping; join the reader.
    let _ = reader_handle.join();

    // Drain any remaining buffered stderr lines.
    while let Ok(line) = rx.try_recv() {
        collected.push_str(&line);
    }

    // Archon writes tracing output to a log file, not stderr. Scrape
    // $XDG_DATA_HOME/archon/logs/*.log for the startup messages.
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
fn memory_disabled_skips_tools_and_graph_injection() {
    if archon_bin().is_none() {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set (binary not built)");
        return;
    }

    let stderr = run_archon_capture_stderr(false);

    assert!(
        stderr.contains(DISABLED_LOG),
        "expected stderr to contain DISABLED log line\n\
         looking for: {DISABLED_LOG:?}\n\
         --- captured stderr ---\n{stderr}\n--- end stderr ---"
    );
    assert!(
        !stderr.contains(ENABLED_LOG),
        "expected stderr NOT to contain ENABLED log line when memory.enabled=false\n\
         found: {ENABLED_LOG:?}\n\
         --- captured stderr ---\n{stderr}\n--- end stderr ---"
    );
}

#[test]
fn memory_enabled_registers_tools_and_injects_graph() {
    if archon_bin().is_none() {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set (binary not built)");
        return;
    }

    let stderr = run_archon_capture_stderr(true);

    assert!(
        stderr.contains(ENABLED_LOG),
        "expected stderr to contain ENABLED log line\n\
         looking for: {ENABLED_LOG:?}\n\
         --- captured stderr ---\n{stderr}\n--- end stderr ---"
    );
    assert!(
        !stderr.contains(DISABLED_LOG),
        "expected stderr NOT to contain DISABLED log line when memory.enabled=true\n\
         found: {DISABLED_LOG:?}\n\
         --- captured stderr ---\n{stderr}\n--- end stderr ---"
    );
}
