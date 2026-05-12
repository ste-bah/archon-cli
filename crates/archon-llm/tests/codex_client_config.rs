use std::collections::BTreeMap;

use archon_llm::auth::CodexCredentials;
use archon_llm::provider::LlmError;
use archon_llm::providers::codex::client::{
    build_codex_headers, build_reasoning_config, clamp_reasoning_effort, validate_spoof_headers,
};
use archon_llm::providers::codex::spoof_default::SpoofConfig;
use archon_llm::providers::{CompatKind, get_native};
use archon_llm::types::Secret;
use chrono::Utc;

fn creds() -> CodexCredentials {
    CodexCredentials {
        access_token: Secret::new("access-token".to_string()),
        refresh_token: Secret::new("refresh-token".to_string()),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        account_id: "acct_123".into(),
    }
}

#[test]
fn headers_include_required_codex_spoof_values() {
    let headers =
        build_codex_headers(&creds(), &SpoofConfig::default(), "session-1").expect("headers build");

    assert_eq!(headers["authorization"], "Bearer access-token");
    assert_eq!(headers["chatgpt-account-id"], "acct_123");
    assert_eq!(headers["originator"], "openclaw");
    assert_eq!(headers["accept"], "text/event-stream");
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["session_id"], "session-1");
    assert_eq!(headers["x-client-request-id"], "session-1");
    assert_eq!(headers["openai-beta"], "responses=experimental");
}

#[test]
fn reserved_header_validation_is_case_insensitive() {
    for key in [
        "Authorization",
        "chatgpt-account-id",
        "content-type",
        "accept",
        "session_id",
        "x-client-request-id",
        "User-Agent",
        "OpenAI-Beta",
    ] {
        let mut spoof = SpoofConfig::default();
        spoof.extra_headers = BTreeMap::from([(key.to_string(), "bad".to_string())]);
        assert!(matches!(
            validate_spoof_headers(&spoof),
            Err(LlmError::Auth(message)) if message.contains("reserved header")
        ));
    }
}

#[test]
fn effort_clamp_matches_codex_rules() {
    assert_eq!(clamp_reasoning_effort("gpt-5.2", "minimal"), "low");
    assert_eq!(clamp_reasoning_effort("gpt-5.3-codex", "minimal"), "low");
    assert_eq!(clamp_reasoning_effort("gpt-5.4/foo", "minimal"), "minimal");
    assert_eq!(clamp_reasoning_effort("gpt-5.1", "xhigh"), "high");
    assert_eq!(
        clamp_reasoning_effort("gpt-5.3-codex-mini", "low"),
        "medium"
    );
    assert_eq!(
        clamp_reasoning_effort("gpt-5.3-codex-mini", "xhigh"),
        "high"
    );
}

#[test]
fn reasoning_config_is_omitted_without_effort() {
    assert!(build_reasoning_config("gpt-5.3-codex", None).is_none());
    let cfg = build_reasoning_config("gpt-5.3-codex-mini", Some("low")).expect("config");
    assert_eq!(cfg.effort.as_deref(), Some("medium"));
    assert_eq!(cfg.summary.as_deref(), Some("auto"));
}

#[test]
fn openai_codex_descriptor_is_in_native_registry() {
    let descriptor = get_native("openai-codex").expect("descriptor");

    assert_eq!(
        descriptor.display_name,
        "OpenAI Codex (ChatGPT Subscription)"
    );
    assert_eq!(descriptor.compat_kind, CompatKind::Native);
    assert_eq!(descriptor.default_model, "gpt-5.4");
    assert!(descriptor.supports.streaming);
    assert!(descriptor.supports.tool_calling);
    assert!(descriptor.supports.vision);
    assert!(!descriptor.supports.embeddings);
}
