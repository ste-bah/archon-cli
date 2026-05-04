use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

fn isolated_env(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let home = tmp.join("home");
    let config = tmp.join("config");
    let logs = tmp.join("logs");
    fs::create_dir_all(home.join(".archon")).expect("home dirs");
    fs::create_dir_all(config.join("archon")).expect("config dirs");
    fs::create_dir_all(&logs).expect("log dirs");
    (home, config, logs)
}

fn write_codex_config(config_root: &Path) {
    fs::write(
        config_root.join("archon").join("config.toml"),
        r#"
[providers.openai-codex.spoof]
originator = "openclaw"
user_agent = "openclaw/test"
client_id = "app_EMoamEEZ73f0CkXaXp7hrann"
openai_beta = "responses=experimental"

[providers.openai-codex.manifest]
fetch_url = "https://invalid.localhost/codex-compat.json"
ttl_seconds = 21600
cache_dir = "~/.archon/cache/codex-compat"
"#,
    )
    .expect("write config");
}

fn write_credentials(home: &Path) {
    fs::write(
        home.join(".archon").join(".credentials.json"),
        r#"{
  "openaiCodexOauth": {
    "accessToken": "codex-access-token",
    "refreshToken": "codex-refresh-token",
    "expiresAt": "2099-01-01T00:00:00Z",
    "accountId": "acct_1234567890"
  }
}"#,
    )
    .expect("write credentials");
}

fn command_with_env(bin: &Path, tmp: &Path) -> Command {
    let (home, config, logs) = isolated_env(tmp);
    let mut cmd = Command::new(bin);
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", &config)
        .env("ARCHON_CONFIG_DIR", config.join("archon"))
        .env("ARCHON_LOG_DIR", logs);
    cmd
}

#[test]
fn auth_status_reports_unauthenticated_providers() {
    let Some(bin) = archon_bin() else {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = command_with_env(&bin, tmp.path())
        .args(["auth", "status"])
        .output()
        .expect("run archon auth status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Anthropic (Claude)"));
    assert!(stdout.contains("Codex (OpenAI ChatGPT subscription)"));
    assert!(stdout.contains("auth login --provider anthropic"));
    assert!(stdout.contains("auth login --provider openai-codex"));
}

#[test]
fn auth_status_respects_codex_kill_switch() {
    let Some(bin) = archon_bin() else {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = command_with_env(&bin, tmp.path())
        .env("ARCHON_CODEX_DISABLED", "1")
        .args(["auth", "status"])
        .output()
        .expect("run archon auth status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("DISABLED via ARCHON_CODEX_DISABLED=1"));
}

#[test]
fn auth_status_redacts_codex_identity_fields() {
    let Some(bin) = archon_bin() else {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let (home, config, logs) = isolated_env(tmp.path());
    write_credentials(&home);
    write_codex_config(&config);

    let output = Command::new(bin)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", &config)
        .env("ARCHON_CONFIG_DIR", config.join("archon"))
        .env("ARCHON_LOG_DIR", logs)
        .args(["auth", "status"])
        .output()
        .expect("run archon auth status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("authenticated as account ***...7890"));
    assert!(stdout.contains("Spoof identity:   from config.toml"));
    assert!(stdout.contains("client-id:      app_EMoamEEZ73..."));
    assert!(!stdout.contains("codex-access-token"));
    assert!(!stdout.contains("codex-refresh-token"));
    assert!(!stdout.contains("acct_1234567890"));
}
