use archon_llm::oauth_codex::CodexOAuthClient;
use base64::Engine;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn jwt(account_id: &str) -> String {
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("{}");
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id
            }
        })
        .to_string(),
    );
    format!("{header}.{payload}.sig")
}

#[tokio::test]
async fn refresh_returns_rotated_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": jwt("acct_refresh"),
            "refresh_token": "refresh-rotated",
            "expires_in": 7200
        })))
        .mount(&server)
        .await;
    let client = CodexOAuthClient::new_with_urls(
        reqwest::Client::new(),
        format!("{}/oauth/authorize", server.uri()),
        format!("{}/oauth/token", server.uri()),
    );

    let creds = client.refresh("old-refresh").await.expect("refresh");
    assert_eq!(creds.account_id, "acct_refresh");
    assert_eq!(creds.refresh_token.expose(), "refresh-rotated");
}

#[tokio::test]
async fn refresh_maps_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string("expired"))
        .mount(&server)
        .await;
    let client = CodexOAuthClient::new_with_urls(
        reqwest::Client::new(),
        format!("{}/oauth/authorize", server.uri()),
        format!("{}/oauth/token", server.uri()),
    );

    let err = client
        .refresh("expired-refresh")
        .await
        .expect_err("refresh should fail");
    assert!(err.to_string().contains("token refresh failed"));
}
