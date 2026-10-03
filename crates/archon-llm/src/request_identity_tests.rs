use crate::anthropic::AnthropicClient;
use crate::auth::AuthProvider;
use crate::identity::{IdentityMode, IdentityProvider};
use crate::provider::{LlmProvider, LlmRequest};
use crate::providers::AnthropicProvider;
use crate::types::Secret;

fn provider(mode: IdentityMode, key: &str, attribution: &str) -> AnthropicProvider {
    AnthropicProvider::new(AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new(key.into())),
        IdentityProvider::new(
            mode,
            attribution.into(),
            attribution.into(),
            attribution.into(),
        ),
        None,
    ))
}

fn spoof() -> IdentityMode {
    IdentityMode::Spoof {
        version: "2.1.0".into(),
        entrypoint: "cli".into(),
        betas: Vec::new(),
        workload: None,
        anti_distillation: false,
    }
}

#[test]
fn wire_system_blocks_are_keyed_but_credentials_and_attribution_are_not() {
    let request = LlmRequest {
        model: "opus".into(),
        messages: vec![serde_json::json!({"role":"user", "content":"critic question"})],
        extra: serde_json::json!({"temperature":0.0}),
        ..Default::default()
    };
    let clean = provider(IdentityMode::Clean, "key-one", "session-one");
    let spoofed = provider(spoof(), "key-one", "session-one");
    let fresh_session = provider(spoof(), "key-two", "session-two");
    let clean_id = clean.request_identity(&request).unwrap();
    let spoof_id = spoofed.request_identity(&request).unwrap();
    assert_ne!(clean_id, spoof_id);
    assert_eq!(spoof_id, fresh_session.request_identity(&request).unwrap());
    assert_eq!(spoof_id.len(), 64);
    let client = spoofed.as_anthropic().unwrap();
    let mut resolved = request;
    spoofed.resolve_request_model(&mut resolved);
    let wire = client.build_request_body(&resolved.into()).unwrap();
    let body: serde_json::Value = serde_json::from_str(&wire).unwrap();
    assert!(
        body["system"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("x-anthropic-billing-header:")
    );
    assert!(
        body["system"][1]["text"]
            .as_str()
            .unwrap()
            .starts_with("You are Claude Code,")
    );
    assert_eq!(spoof_id, super::digest(client.api_url(), body).unwrap());
}
