use std::collections::BTreeMap;
use std::path::PathBuf;

use archon_llm::auth::CodexCredentials;
use archon_llm::provider::{LlmError, LlmProvider, LlmRequest};
use archon_llm::providers::codex::client::{
    CodexProvider, build_codex_headers, build_reasoning_config, clamp_reasoning_effort,
    validate_spoof_headers,
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
fn codex_request_body_stays_responses_shaped_without_anthropic_cache_control() {
    let provider = CodexProvider::new(
        PathBuf::from("/tmp/archon-test-codex-auth.json"),
        SpoofConfig::default(),
        reqwest::Client::new(),
    )
    .expect("provider");
    let request = LlmRequest {
        model: "gpt-5.3-codex".into(),
        messages: vec![serde_json::json!({
            "role": "assistant",
            "content": [{"type": "tool_use", "id": "call_1", "name": "Read", "input": {}}],
        })],
        tools: vec![serde_json::json!({
            "name": "Read",
            "description": "Read a file",
            "input_schema": {"type": "object", "properties": {}},
        })],
        ..LlmRequest::default()
    };

    let body = provider.build_request_body(&request).expect("body");
    let wire = serde_json::to_value(&body).expect("json");

    assert!(wire.get("input").is_some());
    assert!(wire.get("messages").is_none());
    assert!(!wire.to_string().contains("cache_control"));
}

#[test]
fn codex_models_report_known_context_windows() {
    let provider = CodexProvider::new(
        PathBuf::from("/tmp/archon-test-codex-auth.json"),
        SpoofConfig::default(),
        reqwest::Client::new(),
    )
    .expect("provider");
    let models = provider.models();

    assert_eq!(
        models
            .iter()
            .find(|model| model.id == "gpt-5.5")
            .unwrap()
            .context_window,
        1_050_000
    );
    assert_eq!(
        models
            .iter()
            .find(|model| model.id == "gpt-5.3-codex")
            .unwrap()
            .context_window,
        400_000
    );
    assert!(models.iter().all(|model| model.context_window > 0));
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
