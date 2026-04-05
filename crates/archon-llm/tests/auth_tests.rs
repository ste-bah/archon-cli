use archon_llm::auth::{AuthError, AuthProvider, parse_credentials_json, resolve_auth};
use archon_llm::types::Secret;

// -----------------------------------------------------------------------
// Secret wrapper tests
// -----------------------------------------------------------------------

#[test]
fn secret_debug_redacts_value() {
    let s = Secret::new("sk-ant-super-secret-key".to_string());
    let debug = format!("{s:?}");
    assert!(
        !debug.contains("sk-ant"),
        "Secret Debug must not leak value, got: {debug}"
    );
    assert!(
        debug.contains("***"),
        "Secret Debug should show redacted marker, got: {debug}"
    );
}

#[test]
fn secret_expose_returns_value() {
    let s = Secret::new("my-token".to_string());
    assert_eq!(s.expose(), "my-token");
}

#[test]
fn secret_clone_preserves_value() {
    let s = Secret::new("abc".to_string());
    let cloned = s.clone();
    assert_eq!(cloned.expose(), "abc");
}

// -----------------------------------------------------------------------
// Credential JSON parsing
// -----------------------------------------------------------------------

#[test]
fn parse_valid_credentials_json() {
    let json = r#"{
        "claudeAiOauth": {
            "accessToken": "access-123",
            "refreshToken": "refresh-456",
            "expiresAt": "2099-12-31T23:59:59Z",
            "scopes": ["user:profile", "user:inference"],
            "subscriptionType": "pro"
        }
    }"#;

    let creds = parse_credentials_json(json).expect("should parse valid JSON");
    assert_eq!(creds.access_token.expose(), "access-123");
    assert_eq!(creds.refresh_token.expose(), "refresh-456");
    assert_eq!(creds.scopes.len(), 2);
    assert_eq!(creds.subscription_type, "pro");
    assert!(
        !creds.is_expired(),
        "token expiring in 2099 should not be expired"
    );
}

#[test]
fn parse_credentials_epoch_ms_integer() {
    // Real Claude Code format: expiresAt is epoch-ms integer
    let json = r#"{
        "claudeAiOauth": {
            "accessToken": "access-epoch",
            "refreshToken": "refresh-epoch",
            "expiresAt": 4102444799000,
            "scopes": ["user:profile"],
            "subscriptionType": "pro"
        }
    }"#;

    let creds = parse_credentials_json(json).expect("should parse epoch-ms integer");
    assert_eq!(creds.access_token.expose(), "access-epoch");
    assert!(
        !creds.is_expired(),
        "far-future epoch should not be expired"
    );
}

#[test]
fn parse_credentials_expired_epoch_ms() {
    // Epoch-ms from the past
    let json = r#"{
        "claudeAiOauth": {
            "accessToken": "a",
            "refreshToken": "r",
            "expiresAt": 1577836800000,
            "scopes": [],
            "subscriptionType": "free"
        }
    }"#;

    let creds = parse_credentials_json(json).expect("should parse");
    assert!(creds.is_expired(), "past epoch-ms should be expired");
}

#[test]
fn parse_credentials_detects_expired_token() {
    let json = r#"{
        "claudeAiOauth": {
            "accessToken": "access-old",
            "refreshToken": "refresh-old",
            "expiresAt": "2020-01-01T00:00:00Z",
            "scopes": [],
            "subscriptionType": "free"
        }
    }"#;

    let creds = parse_credentials_json(json).expect("should parse");
    assert!(creds.is_expired(), "token from 2020 should be expired");
}

#[test]
fn parse_credentials_detects_near_expiry() {
    // Token expiring in 2 minutes should be treated as expired (5-min buffer)
    let near_future = chrono::Utc::now() + chrono::Duration::minutes(2);
    let json = format!(
        r#"{{
            "claudeAiOauth": {{
                "accessToken": "access-near",
                "refreshToken": "refresh-near",
                "expiresAt": "{}",
                "scopes": [],
                "subscriptionType": "pro"
            }}
        }}"#,
        near_future.to_rfc3339()
    );

    let creds = parse_credentials_json(&json).expect("should parse");
    assert!(
        creds.is_expired(),
        "token expiring within 5-min buffer should be treated as expired"
    );
}

#[test]
fn parse_credentials_missing_claude_ai_oauth_key() {
    let json = r#"{ "someOtherKey": {} }"#;
    let err = parse_credentials_json(json).expect_err("should fail");
    match err {
        AuthError::ParseError(msg) => {
            assert!(
                msg.contains("claudeAiOauth"),
                "error should mention missing key, got: {msg}"
            );
        }
        other => panic!("expected ParseError, got: {other:?}"),
    }
}

#[test]
fn parse_credentials_malformed_json() {
    let err = parse_credentials_json("not json at all").expect_err("should fail");
    assert!(
        matches!(err, AuthError::ParseError(_)),
        "malformed JSON should produce ParseError"
    );
}

#[test]
fn parse_credentials_missing_fields() {
    let json = r#"{
        "claudeAiOauth": {
            "accessToken": "tok"
        }
    }"#;
    let err = parse_credentials_json(json).expect_err("should fail on missing fields");
    assert!(matches!(err, AuthError::ParseError(_)));
}

// -----------------------------------------------------------------------
// Auth resolution priority
// -----------------------------------------------------------------------

#[test]
fn resolve_auth_api_key_wins() {
    let result = resolve_auth(
        Some("sk-ant-key"),
        None,
        None, // no credential file
    );
    let provider = result.expect("should resolve");
    assert!(
        matches!(provider, AuthProvider::ApiKey(_)),
        "API key should take highest priority"
    );
    match &provider {
        AuthProvider::ApiKey(key) => assert_eq!(key.expose(), "sk-ant-key"),
        _ => unreachable!(),
    }
}

#[test]
fn resolve_auth_bearer_token_second() {
    let result = resolve_auth(None, Some("bearer-tok"), None);
    let provider = result.expect("should resolve");
    assert!(
        matches!(provider, AuthProvider::BearerToken(_)),
        "Bearer token should be second priority"
    );
}

#[test]
fn resolve_auth_api_key_over_bearer() {
    let result = resolve_auth(Some("sk-ant-key"), Some("bearer-tok"), None);
    let provider = result.expect("should resolve");
    assert!(
        matches!(provider, AuthProvider::ApiKey(_)),
        "API key should win over bearer token"
    );
}

#[test]
fn resolve_auth_no_credentials_returns_error() {
    let result = resolve_auth(None, None, None);
    let err = result.expect_err("should fail with no credentials");
    match err {
        AuthError::NoCredentials(msg) => {
            assert!(
                msg.contains("ANTHROPIC_API_KEY") || msg.contains("archon login"),
                "error should suggest solutions, got: {msg}"
            );
        }
        other => panic!("expected NoCredentials, got: {other:?}"),
    }
}

// -----------------------------------------------------------------------
// AuthProvider header generation
// -----------------------------------------------------------------------

#[test]
fn auth_provider_api_key_header() {
    let provider = AuthProvider::ApiKey(Secret::new("sk-123".into()));
    let (name, value) = provider.header();
    assert_eq!(name, "x-api-key");
    assert_eq!(value, "sk-123");
}

#[test]
fn auth_provider_bearer_header() {
    let provider = AuthProvider::BearerToken(Secret::new("tok-456".into()));
    let (name, value) = provider.header();
    assert_eq!(name, "Authorization");
    assert_eq!(value, "Bearer tok-456");
}

#[test]
fn auth_provider_oauth_header() {
    let creds_json = r#"{
        "claudeAiOauth": {
            "accessToken": "oauth-access-789",
            "refreshToken": "refresh-xxx",
            "expiresAt": "2099-12-31T23:59:59Z",
            "scopes": [],
            "subscriptionType": "pro"
        }
    }"#;
    let creds = parse_credentials_json(creds_json).expect("parse");
    let provider = AuthProvider::OAuthToken(creds);
    let (name, value) = provider.header();
    assert_eq!(name, "Authorization");
    assert_eq!(value, "Bearer oauth-access-789");
}
