use chrono::{Duration, Utc};

use crate::anthropic::{AnthropicClient, MessageRequest};
use crate::auth::{AuthProvider, CodexCredentials, OAuthCredentials};
use crate::identity::{
    DEFAULT_BETAS, IdentityConfigView, IdentityMode, IdentityProvider, resolve_identity_mode,
};
use crate::types::Secret;

fn spoof_identity() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Spoof {
            version: "2.1.89".to_string(),
            entrypoint: "cli".to_string(),
            betas: DEFAULT_BETAS.iter().map(|beta| beta.to_string()).collect(),
            workload: None,
            anti_distillation: false,
        },
        "session-123".to_string(),
        "device-456".to_string(),
        "account-789".to_string(),
    )
}

fn clean_identity() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Clean,
        "session-123".to_string(),
        "device-456".to_string(),
        "account-789".to_string(),
    )
}

fn api_key_auth() -> AuthProvider {
    AuthProvider::ApiKey(Secret::new("sk-ant-api03-test".to_string()))
}

fn oauth_auth() -> AuthProvider {
    AuthProvider::OAuthToken(OAuthCredentials {
        access_token: Secret::new("sk-ant-oat-test".to_string()),
        refresh_token: Secret::new("refresh".to_string()),
        expires_at: Utc::now() + Duration::hours(1),
        scopes: vec!["user".to_string()],
        subscription_type: "pro".to_string(),
    })
}

fn codex_oauth_auth() -> AuthProvider {
    AuthProvider::CodexOAuthToken(CodexCredentials {
        access_token: Secret::new("codex-access".to_string()),
        refresh_token: Secret::new("codex-refresh".to_string()),
        expires_at: Utc::now() + Duration::hours(1),
        account_id: "acct".to_string(),
    })
}

#[test]
fn default_betas_are_spoof_contract_snapshot() {
    assert_eq!(
        DEFAULT_BETAS,
        &[
            "claude-code-20250219",
            "oauth-2025-04-20",
            "interleaved-thinking-2025-05-14",
            "prompt-caching-scope-2026-01-05",
        ]
    );
}

#[test]
fn anthropic_oauth_and_bearer_select_spoof_but_codex_oauth_does_not() {
    assert!(matches!(
        resolve_identity_mode(&oauth_auth(), false, &IdentityConfigView::default()),
        IdentityMode::Spoof { .. }
    ));

    assert!(matches!(
        resolve_identity_mode(
            &AuthProvider::BearerToken(Secret::new("sk-ant-oat-test".to_string())),
            false,
            &IdentityConfigView::default(),
        ),
        IdentityMode::Spoof { .. }
    ));

    assert!(matches!(
        resolve_identity_mode(&codex_oauth_auth(), false, &IdentityConfigView::default()),
        IdentityMode::Clean
    ));
}

#[test]
fn spoof_headers_match_claude_code_contract() {
    let headers = spoof_identity().request_headers("request-abc");

    assert_eq!(headers.get("x-app").map(String::as_str), Some("cli"));
    assert_eq!(
        headers.get("User-Agent").map(String::as_str),
        Some("claude-cli/2.1.89 (external, cli)")
    );
    assert_eq!(
        headers.get("X-Claude-Code-Session-Id").map(String::as_str),
        Some("session-123")
    );
    assert_eq!(
        headers.get("x-client-request-id").map(String::as_str),
        Some("request-abc")
    );
    assert_eq!(
        headers.get("anthropic-version").map(String::as_str),
        Some("2023-06-01")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );

    let beta_header = headers
        .get("anthropic-beta")
        .expect("spoof mode must emit beta header");
    for beta in DEFAULT_BETAS {
        assert!(
            beta_header.split(',').any(|observed| observed == *beta),
            "missing required beta {beta} in {beta_header}"
        );
    }
}

#[test]
fn spoof_metadata_user_id_is_json_string_with_identity_fields() {
    let metadata = spoof_identity().metadata();
    let user_id = metadata
        .get("user_id")
        .and_then(|value| value.as_str())
        .expect("spoof metadata must encode user_id as a JSON string");
    let parsed: serde_json::Value =
        serde_json::from_str(user_id).expect("user_id metadata should be valid JSON");

    assert_eq!(parsed["device_id"], "device-456");
    assert_eq!(parsed["account_uuid"], "account-789");
    assert_eq!(parsed["session_id"], "session-123");
}

#[test]
fn spoof_request_body_injects_billing_identity_and_metadata() {
    let client = AnthropicClient::new(api_key_auth(), spoof_identity(), None);
    let body = client
        .build_request_body(&MessageRequest {
            messages: vec![serde_json::json!({
                "role": "user",
                "content": "please inspect this repository",
            })],
            ..MessageRequest::default()
        })
        .expect("request body should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("body should be JSON");

    let system = parsed["system"]
        .as_array()
        .expect("spoof request should include system blocks");
    assert!(
        system[0]["text"]
            .as_str()
            .unwrap_or_default()
            .starts_with("x-anthropic-billing-header: cc_version=2.1.89."),
        "first spoof block must be the billing header: {system:?}"
    );
    assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
    assert_eq!(
        system[1]["text"],
        "You are Claude Code, Anthropic's official CLI for Claude."
    );
    assert_eq!(system[1]["cache_control"]["scope"], "org");
    assert!(parsed.get("metadata").is_some());
}

#[test]
fn clean_request_body_does_not_inject_spoof_identity() {
    let client = AnthropicClient::new(api_key_auth(), clean_identity(), None);
    let body = client
        .build_request_body(&MessageRequest {
            messages: vec![serde_json::json!({
                "role": "user",
                "content": "hello",
            })],
            ..MessageRequest::default()
        })
        .expect("request body should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("body should be JSON");

    assert!(parsed.get("system").is_none());
    assert!(parsed.get("metadata").is_none());
}
