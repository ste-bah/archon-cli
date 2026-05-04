use archon_llm::auth::CodexCredentials;
use archon_llm::oauth_codex::decode_account_id_from_jwt;
use archon_llm::types::Secret;
use base64::Engine;
use chrono::Utc;

fn jwt_with_payload(payload: serde_json::Value) -> String {
    let header =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string());
    format!("{header}.{payload}.signature")
}

#[test]
fn valid_jwt_decodes_chatgpt_account_id() {
    let jwt = jwt_with_payload(serde_json::json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "acct_test_123"
        }
    }));

    assert_eq!(
        decode_account_id_from_jwt(&jwt).expect("valid jwt"),
        "acct_test_123"
    );
}

#[test]
fn jwt_decode_ignores_signature_segment() {
    let jwt = jwt_with_payload(serde_json::json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "acct_signature_ignored"
        }
    }));

    assert_eq!(
        decode_account_id_from_jwt(&jwt).expect("signature ignored"),
        "acct_signature_ignored"
    );
}

#[test]
fn missing_claim_returns_error() {
    let jwt = jwt_with_payload(serde_json::json!({"sub": "user"}));

    let err = decode_account_id_from_jwt(&jwt).expect_err("missing claim");
    assert!(err.to_string().contains("chatgpt_account_id"));
}

#[test]
fn codex_credentials_debug_redacts_tokens() {
    let creds = CodexCredentials {
        access_token: Secret::new("access-secret-value".to_string()),
        refresh_token: Secret::new("refresh-secret-value".to_string()),
        expires_at: Utc::now(),
        account_id: "acct_visible".into(),
    };

    let debug = format!("{creds:?}");
    assert!(!debug.contains("access-secret-value"));
    assert!(!debug.contains("refresh-secret-value"));
    assert!(debug.contains("acct_visible"));
}
