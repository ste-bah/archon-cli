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
async fn exchange_code_returns_codex_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": jwt("acct_exchange"),
            "refresh_token": "refresh-new",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;

    let client = CodexOAuthClient::new_with_urls(
        reqwest::Client::new(),
        format!("{}/oauth/authorize", server.uri()),
        format!("{}/oauth/token", server.uri()),
    );
    let creds = client
        .exchange_code("auth-code", "verifier")
        .await
        .expect("exchange succeeds");

    assert_eq!(creds.account_id, "acct_exchange");
    assert_eq!(creds.refresh_token.expose(), "refresh-new");
}

#[tokio::test]
async fn exchange_code_maps_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad code"))
        .mount(&server)
        .await;
    let client = CodexOAuthClient::new_with_urls(
        reqwest::Client::new(),
        format!("{}/oauth/authorize", server.uri()),
        format!("{}/oauth/token", server.uri()),
    );

    let err = client
        .exchange_code("bad-code", "verifier")
        .await
        .expect_err("exchange should fail");
    assert!(err.to_string().contains("token exchange failed"));
}

#[tokio::test]
async fn exchange_code_rejects_non_loopback_http_token_endpoint() {
    let client = CodexOAuthClient::new_with_urls(
        reqwest::Client::new(),
        "https://auth.example.test/oauth/authorize".into(),
        "http://auth.example.test/oauth/token".into(),
    );

    let err = client
        .exchange_code("auth-code", "verifier")
        .await
        .expect_err("exchange should fail before HTTP");
    assert!(err.to_string().contains("must use HTTPS"));
}
