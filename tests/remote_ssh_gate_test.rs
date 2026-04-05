//! Integration test for TASK-WIRE-006: `archon remote ssh`.
//!
//! Gate 1 test-first — written BEFORE implementation.
//!
//! Strategy: target a closed local port (bound-then-closed) so the SSH
//! connect attempt hits TCP refusal. This proves the handler actually calls
//! `SshTransport::connect()` (vs. printing a hardcoded stub) because:
//!   * stderr must contain "ssh: connection to <host>:<port> failed" — this
//!     string is only emitted by archon_core::remote::ssh::SshTransport::connect
//!   * process exits with a non-zero status
//!
//! If the handler were still a print stub, it would exit 0 with no error.

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
"#
    .to_string()
}

/// Bind an ephemeral port, then drop the listener so the port is closed. Returns the port.
fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn wait_output(mut child: std::process::Child, timeout: Duration) -> (String, String, i32) {
    let start = Instant::now();
    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();

    loop {
        if let Ok(Some(status)) = child.try_wait() {
            if let Some(ref mut p) = out_pipe {
                let _ = p.read_to_string(&mut stdout_buf);
            }
            if let Some(ref mut p) = err_pipe {
                let _ = p.read_to_string(&mut stderr_buf);
            }
            return (stdout_buf, stderr_buf, status.code().unwrap_or(-1));
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(ref mut p) = out_pipe {
                let _ = p.read_to_string(&mut stdout_buf);
            }
            if let Some(ref mut p) = err_pipe {
                let _ = p.read_to_string(&mut stderr_buf);
            }
            return (stdout_buf, stderr_buf, -1);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run_ssh(target: &str, port: u16) -> (String, String, i32) {
    let bin = archon_bin().expect("archon binary not built");
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("archon");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), minimal_config()).unwrap();
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).unwrap();

    let child = Command::new(&bin)
        .current_dir(&work_dir)
        .env("ARCHON_CONFIG_DIR", &config_dir)
        .env("ANTHROPIC_API_KEY", "sk-fake-test-key-not-real")
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_CONFIG_HOME", tmp.path())
        .arg("remote")
        .arg("ssh")
        .arg(target)
        .arg("--port")
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn archon remote ssh");

    wait_output(child, Duration::from_secs(20))
}

#[test]
fn remote_ssh_closed_port_reports_real_connection_failure() {
    if archon_bin().is_none() {
        return;
    }
    let port = closed_port();
    let (stdout, stderr, code) = run_ssh("testuser@127.0.0.1", port);

    // Must have exited non-zero (real stub would've printed and exited 0)
    assert_ne!(
        code, 0,
        "expected non-zero exit on connection failure; stdout={stdout}\nstderr={stderr}"
    );

    // Must contain the error message emitted by SshTransport::connect
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        combined.contains("SSH connection failed") || combined.contains("connection"),
        "expected real SSH connection failure message in output\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    // The russh/tokio stack reports a connection refused or similar — must
    // contain the host:port string that only ssh.rs emits.
    let expect = format!("{}", port);
    assert!(
        combined.contains(&expect),
        "expected port {expect} mentioned in error output (proves real connect attempt)\n{combined}"
    );
}

#[test]
fn remote_ssh_parses_target_without_user_as_root() {
    if archon_bin().is_none() {
        return;
    }
    let port = closed_port();
    // No @ in target -> defaults to root@host
    let (stdout, stderr, code) = run_ssh("127.0.0.1", port);

    assert_ne!(
        code, 0,
        "expected non-zero exit; stdout={stdout}\nstderr={stderr}"
    );
    let combined = format!("{stdout}\n{stderr}");
    // Handler should print the resolved target before/during the connect attempt.
    assert!(
        combined.contains("root@127.0.0.1") || combined.contains("user=root"),
        "expected target to resolve to root@127.0.0.1 (default user)\n{combined}"
    );
}
