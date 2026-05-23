use crate::anthropic::{AnthropicClient, MessageRequest};
use crate::anthropic_support::extract_unknown_beta;
use crate::auth::AuthProvider;
use crate::identity::{IdentityMode, IdentityProvider};
use crate::types::Secret;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn make_auth() -> AuthProvider {
    AuthProvider::ApiKey(Secret::new("test-key".to_string()))
}

fn make_identity() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Clean,
        "test-session".to_string(),
        "test-device".to_string(),
        String::new(),
    )
}

#[test]
fn extract_unknown_beta_parses_correctly() {
    let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"Unknown beta flag: xyz-2025-01-01"}}"#;
    let result = extract_unknown_beta(body);
    assert_eq!(result, Some("xyz-2025-01-01".to_string()));
}

#[test]
fn extract_unknown_beta_returns_none_for_unrelated_error() {
    let body =
        r#"{"type":"error","error":{"type":"authentication_error","message":"Invalid API key"}}"#;
    let result = extract_unknown_beta(body);
    assert_eq!(result, None);
}

#[test]
fn extract_unknown_beta_returns_none_for_empty_body() {
    let result = extract_unknown_beta("");
    assert_eq!(result, None);
}

#[test]
fn extract_unknown_beta_handles_beta_with_hyphens() {
    let body =
        r#"{"type":"error","error":{"message":"Unknown beta flag: my-feature-flag-2025-12-31"}}"#;
    let result = extract_unknown_beta(body);
    assert_eq!(result, Some("my-feature-flag-2025-12-31".to_string()));
}

#[tokio::test]
async fn validate_betas_with_empty_candidates_returns_empty() {
    let client = AnthropicClient::new(make_auth(), make_identity(), None);
    let result = client.validate_betas(vec![]).await;
    assert!(
        result.is_empty(),
        "empty candidates should return empty immediately without any API call"
    );
}

#[test]
fn probe_body_structure() {
    let body = serde_json::json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "."}],
        "stream": false,
    });
    assert_eq!(body["model"], "claude-haiku-4-5-20251001");
    assert_eq!(body["max_tokens"], 1);
    assert_eq!(body["stream"], false);
    assert!(body["messages"].is_array());
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"], ".");
}

#[test]
fn custom_api_url_stored() {
    let client = AnthropicClient::new(
        make_auth(),
        make_identity(),
        Some("http://localhost:11434/v1/messages".to_string()),
    );
    assert_eq!(client.api_url(), "http://localhost:11434/v1/messages");
}

#[test]
fn default_api_url_when_none() {
    let client = AnthropicClient::new(make_auth(), make_identity(), None);
    assert_eq!(client.api_url(), "https://api.anthropic.com/v1/messages");
}

#[test]
fn custom_api_url_used_not_constant() {
    let custom_url = "https://my-proxy.example.com/v1/messages";
    let client = AnthropicClient::new(make_auth(), make_identity(), Some(custom_url.to_string()));
    assert_ne!(client.api_url(), "https://api.anthropic.com/v1/messages");
    assert_eq!(client.api_url(), custom_url);
}

#[test]
fn unsupported_speed_and_effort_are_dropped_from_wire_body() {
    let client = AnthropicClient::new(make_auth(), make_identity(), None);
    let request = MessageRequest {
        model: "claude-sonnet-4-6".to_string(),
        messages: vec![serde_json::json!({
            "role": "user",
            "content": "summarize this"
        })],
        speed: Some("fast".to_string()),
        effort: Some("low".to_string()),
        ..MessageRequest::default()
    };

    let body = client
        .build_request_body(&request)
        .expect("request body serializes");
    let body_json: serde_json::Value =
        serde_json::from_str(&body).expect("request body parses as JSON");

    assert_eq!(body_json.get("speed"), None);
    assert_eq!(body_json.get("output_config"), None);
    assert!(
        !body.contains("\"speed\""),
        "unsupported speed knob must not reach Anthropic wire body: {body}"
    );
    assert!(
        !body.contains("\"output_config\""),
        "unsupported effort knob must not reach Anthropic wire body: {body}"
    );
}

#[test]
fn tool_definitions_get_anthropic_cache_control() {
    let client = AnthropicClient::new(make_auth(), make_identity(), None);
    let request = MessageRequest {
        messages: vec![serde_json::json!({"role": "user", "content": "hi"})],
        tools: vec![
            serde_json::json!({
                "name": "Agent",
                "description": "spawn",
                "input_schema": {"type": "object"}
            }),
            serde_json::json!({
                "name": "Read",
                "description": "read",
                "input_schema": {"type": "object"}
            }),
        ],
        ..MessageRequest::default()
    };

    let body = client.build_request_body(&request).unwrap();
    let body_json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(body_json["tools"][0].get("cache_control").is_none());
    assert_eq!(
        body_json["tools"][1]["cache_control"],
        serde_json::json!({"type": "ephemeral"})
    );
}

#[tokio::test]
async fn long_retry_after_returns_without_sleeping() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "8004"))
        .mount(&server)
        .await;
    let client = AnthropicClient::new(
        make_auth(),
        make_identity(),
        Some(format!("{}/v1/messages", server.uri())),
    );

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        client.stream_message(MessageRequest {
            messages: vec![serde_json::json!({"role": "user", "content": "hello"})],
            ..MessageRequest::default()
        }),
    )
    .await
    .expect("long retry-after must not sleep inside the client");

    assert!(matches!(
        result,
        Err(crate::anthropic::ApiError::RateLimited {
            retry_after_secs: 8004
        })
    ));
}

#[tokio::test]
async fn large_rate_limited_body_returns_for_caller_compaction() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "5"))
        .mount(&server)
        .await;
    let client = AnthropicClient::new(
        make_auth(),
        make_identity(),
        Some(format!("{}/v1/messages", server.uri())),
    );

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        client.stream_message(MessageRequest {
            messages: vec![serde_json::json!({
                "role": "user",
                "content": "x".repeat(400_000),
            })],
            ..MessageRequest::default()
        }),
    )
    .await
    .expect("large rate-limited request must not retry the identical body");

    assert!(matches!(
        result,
        Err(crate::anthropic::ApiError::RateLimited {
            retry_after_secs: 5
        })
    ));
}

#[test]
fn cache_control_blocks_stay_within_anthropic_budget() {
    let client = AnthropicClient::new(make_auth(), make_identity(), None);
    let tools: Vec<serde_json::Value> = (0..40)
        .map(|i| {
            serde_json::json!({
                "name": format!("tool_{i}"),
                "description": "x",
                "input_schema": {"type": "object"}
            })
        })
        .collect();
    let request = MessageRequest {
        messages: vec![serde_json::json!({"role": "user", "content": "hi"})],
        tools,
        ..MessageRequest::default()
    };

    let body = client.build_request_body(&request).unwrap();
    let count = body.matches("\"cache_control\"").count();
    assert!(
        count <= 4,
        "serialized request carries {count} cache_control blocks; Anthropic caps at 4"
    );
}
