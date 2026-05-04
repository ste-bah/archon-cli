use archon_llm::oauth_codex::{CodexOAuthClient, decode_account_id_from_jwt};
use base64::Engine;

fn synthetic_jwt(account_id: &str) -> String {
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

fn main() {
    let client = CodexOAuthClient::new(reqwest::Client::new());
    let auth_url = client.build_authorize_url("challenge", "state");
    let account_id = decode_account_id_from_jwt(&synthetic_jwt("acct_smoke")).unwrap_or_default();

    println!(
        "auth_url_contains_client={}",
        auth_url.contains("client_id=")
    );
    println!("decoded_account_id={account_id}");
    assert_eq!(account_id, "acct_smoke");
    println!("OK: codex oauth smoke");
}
