use std::collections::BTreeMap;

use archon_llm::auth::CodexCredentials;
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::codex::client::{CodexProvider, build_codex_headers};
use archon_llm::providers::codex::spoof_default::SpoofConfig;
use archon_llm::types::Secret;
use chrono::Utc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_DIR: &str = "tests/fixtures/codex/captured";

fn fixture_json(name: &str) -> serde_json::Value {
    let path = format!("{FIXTURE_DIR}/{name}");
    let content = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
}

fn fixture_text(name: &str) -> String {
    std::fs::read_to_string(format!("{FIXTURE_DIR}/{name}")).unwrap_or_default()
}

fn write_creds(path: &std::path::Path) {
    let expires_at = (Utc::now() + chrono::Duration::hours(1)).timestamp_millis();
    let content = format!(
        r#"{{
          "openaiCodexOauth": {{
            "accessToken": "access-token",
            "refreshToken": "refresh-token",
            "expiresAt": {expires_at},
            "accountId": "acct_123"
          }}
        }}"#
    );
    let _ = std::fs::write(path, content);
}

fn request() -> LlmRequest {
    LlmRequest {
        model: "gpt-5.4".into(),
        system: vec![serde_json::json!({"type": "text", "text": "You are helpful."})],
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })],
        ..LlmRequest::default()
    }
}

#[tokio::test]
async fn captured_request_shape_matches_openclaw_fixture() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            fixture_text("responses_sse_simple.txt"),
            "text/event-stream",
        ))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap_or_else(|_| std::process::exit(1));
    let cred_path = dir.path().join(".credentials.json");
    write_creds(&cred_path);
    let provider = CodexProvider::new_with_base_url(
        cred_path,
        SpoofConfig::default(),
        reqwest::Client::new(),
        server.uri(),
    )
    .unwrap_or_else(|_| std::process::exit(1));

    let mut rx = provider
        .stream(request())
        .await
        .unwrap_or_else(|_| std::process::exit(1));
    while rx.recv().await.is_some() {}

    let received = server.received_requests().await.unwrap_or_default();
    let codex_posts: Vec<_> = received
        .iter()
        .filter(|req| req.method.as_str() == "POST" && req.url.path() == "/codex/responses")
        .collect();
    assert_eq!(codex_posts.len(), 1);
    let actual = codex_posts[0];
    let captured = fixture_json("responses_request_simple.json");
    assert_eq!(actual.url.path(), captured["path"].as_str().unwrap_or(""));

    let actual_body: serde_json::Value =
        serde_json::from_slice(&actual.body).unwrap_or_else(|_| serde_json::json!({}));
    assert_json_matches(&actual_body, &captured["body"], "$");
}

#[test]
fn captured_header_shape_matches_archon_headers() {
    let expected = fixture_json("responses_request_headers.json");
    let expected = expected.as_object().cloned().unwrap_or_default();
    let creds = CodexCredentials {
        access_token: Secret::new("access-token".into()),
        refresh_token: Secret::new("refresh-token".into()),
        expires_at: Utc::now(),
        account_id: "acct_123".into(),
    };

    let headers = build_codex_headers(&creds, &SpoofConfig::default(), "session-123")
        .unwrap_or_else(|_| reqwest::header::HeaderMap::new());
    let actual_names: Vec<_> = headers
        .keys()
        .map(|name| name.as_str().to_ascii_lowercase())
        .collect();

    for name in expected.keys() {
        assert!(actual_names.contains(&name.to_ascii_lowercase()));
    }
}

#[test]
fn captured_tool_and_reasoning_fixtures_match_translator_shape() {
    let provider = CodexProvider::new_with_base_url(
        std::path::PathBuf::from("/tmp/nonexistent"),
        SpoofConfig::default(),
        reqwest::Client::new(),
        "http://127.0.0.1".into(),
    )
    .unwrap_or_else(|_| std::process::exit(1));

    let tool_req = LlmRequest {
        model: "gpt-5.4".into(),
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "call the tool"}]
        })],
        tools: vec![serde_json::json!({
            "name": "lookup",
            "description": "Lookup a value",
            "input_schema": {
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            }
        })],
        ..LlmRequest::default()
    };
    let tool_body = serde_json::to_value(
        provider
            .build_request_body(&tool_req)
            .unwrap_or_else(|_| std::process::exit(1)),
    )
    .unwrap_or_else(|_| serde_json::json!({}));
    assert_json_matches(
        &tool_body,
        &fixture_json("responses_request_with_tools.json")["body"],
        "$",
    );

    let reasoning_req = LlmRequest {
        model: "gpt-5.4".into(),
        effort: Some("medium".into()),
        reasoning_encrypted: Some("encrypted-blob".into()),
        messages: vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "previous answer"}]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": "continue"}]
            }),
        ],
        ..LlmRequest::default()
    };
    let reasoning_body = serde_json::to_value(
        provider
            .build_request_body(&reasoning_req)
            .unwrap_or_else(|_| std::process::exit(1)),
    )
    .unwrap_or_else(|_| serde_json::json!({}));
    assert_json_matches(
        &reasoning_body,
        &fixture_json("responses_request_with_reasoning.json")["body"],
        "$",
    );
}

fn assert_json_matches(actual: &serde_json::Value, expected: &serde_json::Value, path: &str) {
    if let Some(marker) = expected.as_str().filter(|s| s.starts_with("{{")) {
        assert_placeholder(actual, marker, path);
        return;
    }
    match (actual, expected) {
        (serde_json::Value::Object(actual), serde_json::Value::Object(expected)) => {
            let actual_keys: BTreeMap<_, _> = actual.iter().collect();
            for (key, expected_value) in expected {
                assert!(
                    actual_keys.contains_key(key),
                    "missing key {path}.{key}; actual={actual:?}"
                );
                assert_json_matches(&actual[key], expected_value, &format!("{path}.{key}"));
            }
        }
        (serde_json::Value::Array(actual), serde_json::Value::Array(expected)) => {
            assert_eq!(actual.len(), expected.len(), "array length at {path}");
            for (idx, expected_value) in expected.iter().enumerate() {
                assert_json_matches(&actual[idx], expected_value, &format!("{path}[{idx}]"));
            }
        }
        _ => assert_eq!(actual, expected, "value mismatch at {path}"),
    }
}

fn assert_placeholder(actual: &serde_json::Value, marker: &str, path: &str) {
    let Some(value) = actual.as_str() else {
        assert!(
            actual.as_str().is_some(),
            "{path} expected string placeholder {marker}, got {actual:?}"
        );
        return;
    };
    match marker {
        "{{UUID}}" => assert!(!value.is_empty()),
        "{{REASONING_ENCRYPTED}}" => assert!(!value.is_empty()),
        other => assert!(false, "unsupported placeholder {other} at {path}"),
    }
}
