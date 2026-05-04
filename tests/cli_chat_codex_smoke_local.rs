use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

fn write_config(config_root: &Path) {
    let config_dir = config_root.join("archon");
    let _ = fs::create_dir_all(&config_dir);
    let _ = fs::write(
        config_dir.join("config.toml"),
        r#"
[providers.openai-codex.spoof]
originator = "openclaw"
user_agent = "openclaw/test"
client_id = "app_EMoamEEZ73f0CkXaXp7hrann"
openai_beta = "responses=experimental"
"#,
    );
}

fn write_credentials(home: &Path) {
    let archon_dir = home.join(".archon");
    let _ = fs::create_dir_all(&archon_dir);
    let expires_at = (Utc::now() + chrono::Duration::hours(1)).timestamp_millis();
    let content = format!(
        r#"{{
          "openaiCodexOauth": {{
            "accessToken": "access-token",
            "refreshToken": "refresh-token",
            "expiresAt": {expires_at},
            "accountId": "acct_123"
          }}
        }}"#
    );
    let _ = fs::write(archon_dir.join(".credentials.json"), content);
}

#[tokio::test]
async fn chat_codex_no_stream_works_against_wiremock() {
    let Some(bin) = archon_bin() else {
        eprintln!("skipping: CARGO_BIN_EXE_archon not set");
        return;
    };

    let server = MockServer::start().await;
    let sse = [
        r#"data: {"type":"response.created","response":{"id":"resp_1","status":"in_progress","model":"gpt-5.4"}}"#,
        r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_1","status":"in_progress","role":"assistant","content":[]}}"#,
        r#"data: {"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"ARCHON_SMOKE_OK_42"}"#,
        r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","model":"gpt-5.4","usage":{"input_tokens":8,"output_tokens":5,"total_tokens":13}}}"#,
        "data: [DONE]",
    ]
    .join("\n\n")
        + "\n\n";
    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::exit(1));
    let home = tmp.path().join("home");
    let config = tmp.path().join("config");
    let logs = tmp.path().join("logs");
    let _ = fs::create_dir_all(&logs);
    write_credentials(&home);
    write_config(&config);

    let output = Command::new(bin)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config)
        .env("ARCHON_CONFIG_DIR", config.join("archon"))
        .env("ARCHON_LOG_DIR", &logs)
        .env("ARCHON_CODEX_BASE_URL", server.uri())
        .args([
            "chat",
            "--provider",
            "openai-codex",
            "--model",
            "gpt-5.4",
            "--no-stream",
            "--max-tokens",
            "16",
            "Echo the literal string ARCHON_SMOKE_OK_42 verbatim, no other text.",
        ])
        .output()
        .unwrap_or_else(|_| std::process::exit(1));

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("ARCHON_SMOKE_OK_42"),
        "stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
}
