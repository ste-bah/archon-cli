use crate::anthropic::{AnthropicClient, MessageRequest};
use crate::anthropic_support::extract_unknown_beta;
use crate::auth::AuthProvider;
use crate::identity::{IdentityMode, IdentityProvider};
use crate::types::Secret;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub(super) fn make_auth() -> AuthProvider {
    AuthProvider::ApiKey(Secret::new("test-key".to_string()))
}

pub(super) fn make_identity() -> IdentityProvider {
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

/// Build a wire body for `model` at `effort` and return it parsed.
fn wire_body(model: &str, effort: Option<&str>, speed: Option<&str>) -> serde_json::Value {
    let client = AnthropicClient::new(make_auth(), make_identity(), None);
    let request = MessageRequest {
        model: model.to_string(),
        messages: vec![serde_json::json!({
            "role": "user",
            "content": "summarize this"
        })],
        speed: speed.map(str::to_string),
        effort: effort.map(str::to_string),
        ..MessageRequest::default()
    };
    let body = client
        .build_request_body(&request)
        .expect("request body serializes");
    serde_json::from_str(&body).expect("request body parses as JSON")
}

/// #123: `supports_speed` was a stub returning `false` for every model, so
/// fast mode was silently dropped on the wire while `/fast` still reported
/// success. It is now sent by default, like effort.
#[test]
fn speed_reaches_wire_body_by_default() {
    let body = wire_body("claude-sonnet-4-6", None, Some("fast"));
    assert_eq!(
        body["speed"], "fast",
        "fast mode must reach the wire: {body}"
    );
}

/// #123: the effort gate was a hand-maintained substring allowlist
/// (`opus-4` / `sonnet-5` / `fable-5`) that silently dropped the knob on every
/// model not named in it — `sonnet-4-6` and `opus-5` included. It is now a
/// family check, so any current Sonnet or Opus id carries the knob.
#[test]
fn effort_reaches_wire_body_for_all_sonnet_and_opus_models() {
    for model in [
        "claude-sonnet-4-6",
        "claude-sonnet-5",
        "claude-opus-4-8",
        "claude-opus-5",
        "claude-fable-5",
        // Provider-prefixed ids must resolve the same way.
        "us.anthropic.claude-sonnet-4-6-v1:0",
        "claude-sonnet-4-6@20250514",
    ] {
        let body = wire_body(model, Some("low"), None);
        assert_eq!(
            body["output_config"]["effort"], "low",
            "effort should reach the wire for {model}: {body}"
        );
    }
}

/// #123: no model allowlist at all. Any model gets the knob until the API
/// says otherwise, so an unrecognised or brand-new id is not silently
/// downgraded the way `opus-5` was.
#[test]
fn effort_is_model_agnostic_by_default() {
    for model in [
        "claude-haiku-4-5-20251001",
        "some-unreleased-future-model",
        "gpt-4o",
    ] {
        let body = wire_body(model, Some("low"), None);
        assert_eq!(
            body["output_config"]["effort"], "low",
            "no allowlist should gate {model}: {body}"
        );
    }
}

/// A rejection body that names the effort knob switches it off for that model
/// only, and only after the API has actually said so.
#[test]
fn effort_is_disabled_for_a_model_after_the_api_rejects_it() {
    let model = "claude-effort-rejecting-test-model";
    assert_eq!(
        wire_body(model, Some("low"), None)["output_config"]["effort"],
        "low"
    );

    assert!(
        archon_llm_effort_reject(model),
        "first rejection should be recorded"
    );
    assert_eq!(
        wire_body(model, Some("low"), None).get("output_config"),
        None,
        "knob must stay off for this model once rejected"
    );
    assert!(
        !archon_llm_effort_reject(model),
        "a repeat rejection must not trigger another retry"
    );

    // A different model is unaffected — the state is per-model, not global.
    assert_eq!(
        wire_body("claude-unrelated-test-model", Some("low"), None)["output_config"]["effort"],
        "low"
    );
}

/// Thin wrapper so the test reads as "the API rejected it" rather than as an
/// internal function call.
fn archon_llm_effort_reject(model: &str) -> bool {
    crate::anthropic_support::mark_knob_unsupported(model, crate::anthropic_support::EFFORT_KNOB)
}

/// Only a 400 that actually blames a knob switches it off, and only a knob the
/// request carried.
#[test]
fn rejected_knob_identifies_the_blamed_knob_only() {
    use crate::anthropic_support::{EFFORT_KNOB, SPEED_KNOB, rejected_knob};

    let with_both = MessageRequest {
        model: "claude-knob-detection-test".to_string(),
        effort: Some("low".to_string()),
        speed: Some("fast".to_string()),
        ..MessageRequest::default()
    };

    assert_eq!(
        rejected_knob(
            &with_both,
            r#"{"error":{"message":"Unknown beta flag: effort-2025-11-24"}}"#
        ),
        Some(EFFORT_KNOB)
    );
    assert_eq!(
        rejected_knob(
            &with_both,
            r#"{"error":{"message":"Unknown beta flag: fast-mode-2026-02-01"}}"#
        ),
        Some(SPEED_KNOB)
    );
    assert_eq!(
        rejected_knob(
            &with_both,
            r#"{"error":{"message":"prompt is too long: 300000 tokens > 200000"}}"#
        ),
        None,
        "an unrelated 400 must not switch a knob off"
    );
    assert_eq!(
        rejected_knob(
            &with_both,
            r#"{"error":{"message":"Unknown beta flag: some-other-beta"}}"#
        ),
        None,
        "a different unknown beta must not switch these knobs off"
    );

    // A knob the request never carried is never blamed.
    let effort_only = MessageRequest {
        model: "claude-knob-detection-test".to_string(),
        effort: Some("low".to_string()),
        ..MessageRequest::default()
    };
    assert_eq!(
        rejected_knob(
            &effort_only,
            r#"{"error":{"message":"Unknown beta flag: fast-mode-2026-02-01"}}"#
        ),
        None
    );
}

/// #123: the core layer now always sends a concrete level, so `high` and `max`
/// arrive here as strings rather than as an absent field. Anthropic's ladder
/// stops at high and omitting the field already means high, so both must be
/// dropped — reproducing the pre-#123 wire bytes exactly.
#[test]
fn high_and_max_are_omitted_because_absent_already_means_high() {
    for level in ["high", "max"] {
        let body = wire_body("claude-opus-5", Some(level), None);
        assert_eq!(
            body.get("output_config"),
            None,
            "'{level}' must be expressed by omission, not sent: {body}"
        );
    }
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
