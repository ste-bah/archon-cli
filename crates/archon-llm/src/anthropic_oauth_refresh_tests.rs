use chrono::{Duration as ChronoDuration, Utc};
use serial_test::serial;
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::anthropic::{AnthropicClient, MessageRequest};
use crate::auth::{AuthProvider, OAuthCredentials};
use crate::identity::{IdentityMode, IdentityProvider};
use crate::tokens;
use crate::types::Secret;

fn identity() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Clean,
        "session-refresh".to_string(),
        "device-refresh".to_string(),
        String::new(),
    )
}

fn oauth(access: &str, refresh: &str, expires_at: chrono::DateTime<Utc>) -> OAuthCredentials {
    OAuthCredentials {
        access_token: Secret::new(access.to_string()),
        refresh_token: Secret::new(refresh.to_string()),
        expires_at,
        scopes: vec!["user:inference".to_string()],
        subscription_type: "pro".to_string(),
    }
}

fn request() -> MessageRequest {
    MessageRequest {
        model: "claude-sonnet-4-6".to_string(),
        max_tokens: 16,
        messages: vec![serde_json::json!({"role": "user", "content": "hello"})],
        ..MessageRequest::default()
    }
}

fn sse_ok() -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_string("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n")
}

fn write_credentials(dir: &TempDir, creds: &OAuthCredentials) -> std::path::PathBuf {
    let path = dir.path().join(".credentials.json");
    let json = serde_json::json!({
        "claudeAiOauth": {
            "accessToken": creds.access_token.expose(),
            "refreshToken": creds.refresh_token.expose(),
            "expiresAt": creds.expires_at.timestamp_millis(),
            "scopes": creds.scopes,
            "subscriptionType": creds.subscription_type,
        }
    });
    std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap()).unwrap();
    path
}

async fn mount_refresh(server: &MockServer, refresh: &str, access: &str, rotated: &str) {
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains(format!("refresh_token={refresh}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": access,
            "refresh_token": rotated,
            "expires_in": 7200,
            "scope": "user:inference"
        })))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
#[serial]
async fn expired_oauth_refreshes_before_stream_request() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let expired = oauth(
        "expired-access",
        "refresh-old",
        Utc::now() - ChronoDuration::hours(1),
    );
    let credentials_path = write_credentials(&dir, &expired);
    let _path_guard = tokens::set_credentials_path_for_tests(credentials_path);
    let _endpoint_guard =
        tokens::set_token_endpoint_for_tests(format!("{}/oauth/token", server.uri()));

    mount_refresh(&server, "refresh-old", "fresh-access", "refresh-rotated").await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer fresh-access"))
        .respond_with(sse_ok())
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::new(
        AuthProvider::OAuthToken(expired),
        identity(),
        Some(server.uri()),
    );
    client.stream_message(request()).await.unwrap();
}

#[tokio::test]
#[serial]
async fn valid_oauth_uses_fast_path_without_refresh_network_call() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let valid = oauth(
        "valid-access",
        "refresh-valid",
        Utc::now() + ChronoDuration::hours(2),
    );
    let credentials_path = write_credentials(&dir, &valid);
    let _path_guard = tokens::set_credentials_path_for_tests(credentials_path);
    let _endpoint_guard =
        tokens::set_token_endpoint_for_tests(format!("{}/oauth/token", server.uri()));

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer valid-access"))
        .respond_with(sse_ok())
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::new(
        AuthProvider::OAuthToken(valid),
        identity(),
        Some(server.uri()),
    );
    client.stream_message(request()).await.unwrap();
}

#[tokio::test]
#[serial]
async fn api_key_stream_request_never_attempts_oauth_refresh() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let credentials_path = write_credentials(
        &dir,
        &oauth(
            "unused-access",
            "unused-refresh",
            Utc::now() - ChronoDuration::hours(1),
        ),
    );
    let _path_guard = tokens::set_credentials_path_for_tests(credentials_path);
    let _endpoint_guard =
        tokens::set_token_endpoint_for_tests(format!("{}/oauth/token", server.uri()));

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(header("x-api-key", "test-api-key"))
        .respond_with(sse_ok())
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("test-api-key".to_string())),
        identity(),
        Some(server.uri()),
    );
    client.stream_message(request()).await.unwrap();
}

#[tokio::test]
#[serial]
async fn oauth_401_forces_refresh_and_retries_once() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let valid = oauth(
        "stale-access",
        "refresh-stale",
        Utc::now() + ChronoDuration::hours(2),
    );
    let credentials_path = write_credentials(&dir, &valid);
    let _path_guard = tokens::set_credentials_path_for_tests(credentials_path);
    let _endpoint_guard =
        tokens::set_token_endpoint_for_tests(format!("{}/oauth/token", server.uri()));

    mount_refresh(&server, "refresh-stale", "retry-access", "refresh-retry").await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer stale-access"))
        .respond_with(ResponseTemplate::new(401).set_body_string("expired"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer retry-access"))
        .respond_with(sse_ok())
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::new(
        AuthProvider::OAuthToken(valid),
        identity(),
        Some(server.uri()),
    );
    client.stream_message(request()).await.unwrap();
}

#[tokio::test]
#[serial]
async fn repeated_oauth_401_returns_error_without_refresh_loop() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let valid = oauth(
        "stale-access",
        "refresh-stale",
        Utc::now() + ChronoDuration::hours(2),
    );
    let credentials_path = write_credentials(&dir, &valid);
    let _path_guard = tokens::set_credentials_path_for_tests(credentials_path);
    let _endpoint_guard =
        tokens::set_token_endpoint_for_tests(format!("{}/oauth/token", server.uri()));

    mount_refresh(&server, "refresh-stale", "retry-access", "refresh-retry").await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer stale-access"))
        .respond_with(ResponseTemplate::new(401).set_body_string("expired"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer retry-access"))
        .respond_with(ResponseTemplate::new(401).set_body_string("still expired"))
        .expect(1)
        .mount(&server)
        .await;

    let client = AnthropicClient::new(
        AuthProvider::OAuthToken(valid),
        identity(),
        Some(server.uri()),
    );
    let err = client
        .stream_message(request())
        .await
        .expect_err("second 401 should be returned");
    assert!(err.to_string().contains("still expired"));
}
