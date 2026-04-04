use std::fs;
use std::path::PathBuf;

use archon_llm::tokens::{
    credentials_path, read_credentials_locked, write_credentials_atomic,
};
use archon_llm::auth::parse_credentials_json;
use archon_llm::types::Secret;

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir()
        .join("archon-tokens-test")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn cleanup(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

fn write_test_credentials(path: &std::path::Path, expires_ms: i64) {
    let json = serde_json::json!({
        "claudeAiOauth": {
            "accessToken": "test-access-token",
            "refreshToken": "test-refresh-token",
            "expiresAt": expires_ms,
            "scopes": ["user:profile"],
            "subscriptionType": "pro"
        }
    });
    fs::write(path, serde_json::to_string_pretty(&json).expect("serialize"))
        .expect("write test credentials");
}

#[test]
fn credentials_path_points_to_claude_dir() {
    let path = credentials_path();
    let s = path.to_string_lossy();
    assert!(
        s.contains(".claude") && s.contains(".credentials.json"),
        "expected .claude/.credentials.json, got: {s}"
    );
}

#[test]
fn read_credentials_locked_reads_valid_file() {
    let dir = temp_dir();
    let cred_file = dir.join(".credentials.json");

    // Far future: 2099
    write_test_credentials(&cred_file, 4102444799000);

    let (creds, _mtime) =
        read_credentials_locked(&cred_file).expect("should read locked");

    assert_eq!(creds.access_token.expose(), "test-access-token");
    assert_eq!(creds.refresh_token.expose(), "test-refresh-token");
    assert!(!creds.is_expired());

    cleanup(&dir);
}

#[test]
fn read_credentials_locked_missing_file_errors() {
    let dir = temp_dir();
    let cred_file = dir.join("nonexistent.json");

    let result = read_credentials_locked(&cred_file);
    assert!(result.is_err());

    cleanup(&dir);
}

#[test]
fn write_credentials_atomic_creates_file() {
    let dir = temp_dir();
    let cred_file = dir.join("new-creds.json");

    let creds = parse_credentials_json(
        r#"{
            "claudeAiOauth": {
                "accessToken": "new-access",
                "refreshToken": "new-refresh",
                "expiresAt": 4102444799000,
                "scopes": ["user:profile"],
                "subscriptionType": "pro"
            }
        }"#,
    )
    .expect("parse");

    write_credentials_atomic(&cred_file, &creds).expect("write should succeed");

    assert!(cred_file.exists(), "credential file should be created");

    // Verify content round-trips
    let content = fs::read_to_string(&cred_file).expect("read back");
    let re_parsed = parse_credentials_json(&content).expect("re-parse");
    assert_eq!(re_parsed.access_token.expose(), "new-access");
    assert_eq!(re_parsed.refresh_token.expose(), "new-refresh");

    cleanup(&dir);
}

#[test]
fn write_credentials_atomic_sets_permissions() {
    let dir = temp_dir();
    let cred_file = dir.join("perms-creds.json");

    let creds = parse_credentials_json(
        r#"{
            "claudeAiOauth": {
                "accessToken": "a",
                "refreshToken": "r",
                "expiresAt": 4102444799000,
                "scopes": [],
                "subscriptionType": "free"
            }
        }"#,
    )
    .expect("parse");

    write_credentials_atomic(&cred_file, &creds).expect("write");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&cred_file)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "file should be 0600, got {:o}", mode);
    }

    cleanup(&dir);
}

#[test]
fn write_credentials_atomic_no_temp_file_left() {
    let dir = temp_dir();
    let cred_file = dir.join("clean-creds.json");

    let creds = parse_credentials_json(
        r#"{
            "claudeAiOauth": {
                "accessToken": "a",
                "refreshToken": "r",
                "expiresAt": 4102444799000,
                "scopes": [],
                "subscriptionType": "free"
            }
        }"#,
    )
    .expect("parse");

    write_credentials_atomic(&cred_file, &creds).expect("write");

    let tmp_file = cred_file.with_extension("json.tmp");
    assert!(
        !tmp_file.exists(),
        "temp file should be cleaned up after rename"
    );

    cleanup(&dir);
}

#[test]
fn concurrent_reads_dont_block() {
    let dir = temp_dir();
    let cred_file = dir.join("concurrent-creds.json");
    write_test_credentials(&cred_file, 4102444799000);

    // Read twice in sequence (not truly concurrent, but verifies no deadlock)
    let (c1, _) = read_credentials_locked(&cred_file).expect("read 1");
    let (c2, _) = read_credentials_locked(&cred_file).expect("read 2");

    assert_eq!(c1.access_token.expose(), c2.access_token.expose());

    cleanup(&dir);
}
