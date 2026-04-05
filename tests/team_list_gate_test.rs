//! Integration test for TASK-WIRE-005: `archon team list`.
//!
//! Gate 1 test-first — written BEFORE implementation.
//!
//! Assertions:
//!   * `archon team list` with no `teams/` dir prints "No teams found"
//!   * `archon team list` with 2 team subdirs containing team.json prints
//!     both team ids AND names
//!   * Empty `teams/` directory prints "No teams found"

use std::path::PathBuf;
use std::process::{Command, Stdio};

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

fn run_team_list(work_dir: &std::path::Path, tmp: &std::path::Path) -> (String, String, i32) {
    let bin = archon_bin().expect("archon binary not built");
    let config_dir = tmp.join("archon");
    std::fs::create_dir_all(&config_dir).expect("create archon config dir");
    std::fs::write(config_dir.join("config.toml"), minimal_config()).expect("write config.toml");

    let output = Command::new(&bin)
        .current_dir(work_dir)
        .env("ARCHON_CONFIG_DIR", &config_dir)
        .env("ANTHROPIC_API_KEY", "sk-fake-test-key-not-real")
        .env("XDG_DATA_HOME", tmp.join("data"))
        .env("XDG_CACHE_HOME", tmp.join("cache"))
        .env("XDG_CONFIG_HOME", tmp)
        .arg("team")
        .arg("list")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run archon team list");

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

fn write_team(work_dir: &std::path::Path, id: &str, name: &str) {
    let team_dir = work_dir.join("teams").join(id);
    std::fs::create_dir_all(&team_dir).expect("create team dir");
    let team_json = format!(r#"{{"id":"{id}","name":"{name}","members":[]}}"#);
    std::fs::write(team_dir.join("team.json"), team_json).expect("write team.json");
}

#[test]
fn team_list_no_teams_dir_prints_empty() {
    if archon_bin().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).expect("create work dir");
    // No teams/ subdirectory exists
    let (stdout, stderr, code) = run_team_list(&work_dir, tmp.path());
    assert_eq!(code, 0, "should exit 0; stderr:\n{stderr}");
    assert!(
        stdout.contains("No teams found"),
        "expected 'No teams found' in stdout:\n{stdout}\n--- stderr ---\n{stderr}"
    );
}

#[test]
fn team_list_empty_teams_dir_prints_empty() {
    if archon_bin().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(work_dir.join("teams")).expect("create empty teams/");
    let (stdout, stderr, code) = run_team_list(&work_dir, tmp.path());
    assert_eq!(code, 0, "should exit 0; stderr:\n{stderr}");
    assert!(
        stdout.contains("No teams found"),
        "expected 'No teams found' in stdout:\n{stdout}\n--- stderr ---\n{stderr}"
    );
}

#[test]
fn team_list_with_teams_prints_ids_and_names() {
    if archon_bin().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).expect("create work dir");
    write_team(&work_dir, "alpha", "Alpha Squad");
    write_team(&work_dir, "beta", "Beta Ops");

    let (stdout, stderr, code) = run_team_list(&work_dir, tmp.path());
    assert_eq!(code, 0, "should exit 0; stderr:\n{stderr}");
    assert!(
        stdout.contains("alpha"),
        "expected team id 'alpha' in stdout:\n{stdout}\n--- stderr ---\n{stderr}"
    );
    assert!(
        stdout.contains("Alpha Squad"),
        "expected team name 'Alpha Squad' in stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("beta"),
        "expected team id 'beta' in stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("Beta Ops"),
        "expected team name 'Beta Ops' in stdout:\n{stdout}"
    );
}
