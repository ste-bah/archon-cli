//! Sampling survives generic-to-wire conversion without affecting defaults.
use archon_llm::anthropic::{AnthropicClient, MessageRequest};
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::LlmRequest;
use archon_llm::types::Secret;

#[test]
fn explicit_temperature_reaches_messages_wire_and_round_trips() {
    let client = AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("test-key".into())),
        IdentityProvider::new(IdentityMode::Clean, "s".into(), "d".into(), String::new()),
        Some("http://127.0.0.1:1/v1/messages".into()),
    );
    for temperature in [None, Some(0.0)] {
        let request = LlmRequest {
            extra: temperature
                .map(|v| serde_json::json!({"temperature":v}))
                .unwrap_or_default(),
            ..Default::default()
        };
        let message: MessageRequest = request.into();
        let body: serde_json::Value =
            serde_json::from_str(&client.build_request_body(&message).unwrap()).unwrap();
        assert_eq!(
            body.get("temperature").and_then(|v| v.as_f64()),
            temperature
        );
        let round_trip: LlmRequest = message.into();
        assert_eq!(
            round_trip.extra.get("temperature").and_then(|v| v.as_f64()),
            temperature
        );
    }
}
