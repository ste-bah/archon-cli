use std::fs;
use std::path::PathBuf;

use archon_llm::auth::{
    CodexCredentials, parse_codex_cli_credentials_json, parse_codex_credentials_json,
    parse_credentials_json,
};
use archon_llm::tokens_codex::{read_codex_credentials_locked, write_codex_credentials_atomic};
use archon_llm::types::Secret;
use base64::Engine;
use chrono::{TimeZone, Utc};

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir()
        .join("archon-codex-credential-test")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn sample_codex() -> CodexCredentials {
    CodexCredentials {
        access_token: Secret::new("codex-access".into()),
        refresh_token: Secret::new("codex-refresh".into()),
        expires_at: Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).unwrap(),
        account_id: "acct_codex".into(),
    }
}

fn unsigned_jwt(payload: serde_json::Value) -> String {
    let header = serde_json::json!({ "alg": "none" });
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&header).expect("encode header"));
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&payload).expect("encode payload"));
    format!("{header}.{payload}.")
}

#[test]
fn parse_codex_credentials_rfc3339() {
    let json = r#"{
        "openaiCodexOauth": {
            "accessToken": "access",
            "refreshToken": "refresh",
            "expiresAt": "2099-01-01T00:00:00Z",
            "accountId": "acct_rfc3339"
        }
    }"#;

    let creds = parse_codex_credentials_json(json).expect("parse codex");
    assert_eq!(creds.account_id, "acct_rfc3339");
    assert_eq!(creds.access_token.expose(), "access");
}

#[test]
fn parse_codex_credentials_epoch_ms() {
    let json = r#"{
        "openaiCodexOauth": {
            "accessToken": "access",
            "refreshToken": "refresh",
            "expiresAt": 4070908800000,
            "accountId": "acct_epoch"
        }
    }"#;

    let creds = parse_codex_credentials_json(json).expect("parse epoch");
    assert_eq!(creds.account_id, "acct_epoch");
}

#[test]
fn parse_codex_cli_credentials_reads_official_auth_json_shape() {
    let exp = Utc::now() + chrono::Duration::hours(1);
    let access_token = unsigned_jwt(serde_json::json!({
        "exp": exp.timestamp(),
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "acct_from_jwt"
        }
    }));
    let json = serde_json::json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "access_token": access_token,
            "refresh_token": "refresh",
            "account_id": "acct_from_file"
        }
    })
    .to_string();

    let creds = parse_codex_cli_credentials_json(&json).expect("parse Codex CLI auth");
    assert_eq!(creds.account_id, "acct_from_file");
    assert_eq!(creds.refresh_token.expose(), "refresh");
    assert!(creds.expires_at > Utc::now());
}

#[test]
fn write_codex_preserves_claude_credentials() {
    let dir = temp_dir();
    let path = dir.join(".credentials.json");
    fs::write(
        &path,
        r#"{
          "claudeAiOauth": {
            "accessToken": "claude-access",
            "refreshToken": "claude-refresh",
            "expiresAt": "2099-01-01T00:00:00Z",
            "scopes": ["user:profile"],
            "subscriptionType": "pro"
          }
        }"#,
    )
    .expect("seed credentials");

    write_codex_credentials_atomic(&path, &sample_codex()).expect("write codex");
    let content = fs::read_to_string(&path).expect("read merged credentials");

    let claude = parse_credentials_json(&content).expect("claude preserved");
    let codex = parse_codex_credentials_json(&content).expect("codex present");
    assert_eq!(claude.access_token.expose(), "claude-access");
    assert_eq!(codex.account_id, "acct_codex");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn read_codex_credentials_locked_reads_written_file() {
    let dir = temp_dir();
    let path = dir.join(".credentials.json");
    write_codex_credentials_atomic(&path, &sample_codex()).expect("write codex");

    let (creds, _mtime) = read_codex_credentials_locked(&path).expect("read locked");
    assert_eq!(creds.account_id, "acct_codex");
    let _ = fs::remove_dir_all(dir);
}
