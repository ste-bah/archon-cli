use archon_llm::oauth_codex::decode_account_id_from_jwt;

const FIXTURE_DIR: &str = "tests/fixtures/codex/captured";

fn fixture_json(name: &str) -> serde_json::Value {
    let content = std::fs::read_to_string(format!("{FIXTURE_DIR}/{name}")).unwrap_or_default();
    serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
}

#[test]
fn oauth_token_exchange_fixture_uses_openclaw_safe_placeholders() {
    let request = fixture_json("oauth_token_exchange_request.json");
    assert_eq!(request["path"], "/oauth/token");
    assert_eq!(request["body"]["grant_type"], "authorization_code");
    assert_eq!(request["body"]["code"], "{{OAUTH_CODE}}");
    assert_eq!(request["body"]["code_verifier"], "{{OAUTH_CODE_VERIFIER}}");
    assert_eq!(request["body"]["client_id"], "{{CLIENT_ID}}");
}

#[test]
fn synthetic_jwt_decodes_to_test_account() {
    let jwt = std::fs::read_to_string("tests/fixtures/codex/synthetic_jwt.txt")
        .unwrap_or_default()
        .trim()
        .to_string();
    let account_id = decode_account_id_from_jwt(&jwt).unwrap_or_default();

    assert_eq!(account_id, "test-account-12345");
}

#[test]
fn oauth_response_fixture_contains_no_live_secret_values() {
    let response = fixture_json("oauth_token_exchange_response.json");
    assert_eq!(response["access_token"], "{{ACCESS_TOKEN}}");
    assert_eq!(response["refresh_token"], "{{REFRESH_TOKEN}}");
    assert_eq!(response["id_token"], "{{SYNTHETIC_JWT}}");
}
