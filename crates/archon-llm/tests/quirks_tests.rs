//! TASK-AGS-705 Gate 1: integration tests for `ProviderQuirks` population
//! and quirk-driven branching in `OpenAiCompatProvider`.
//!
//! Written BEFORE implementation. These pin the public behavior so the
//! impl has a compile-and-pass target.
//!
//! TASK-AGS-705 DESIGN DECISION: `ProviderQuirks` is `#[serde(skip)]` on
//! `ProviderDescriptor` because quirks are an internal implementation
//! detail — they are NOT user-configurable via TOML/YAML. This lets the
//! struct use `&'static [&'static str]` for `ignore_response_fields`
//! so that all quirk constants (`DEFAULT`, `GROQ`, `DEEPSEEK`, `MISTRAL`)
//! are genuine Rust `const` expressions with zero runtime allocation.
//!
//! Spec validation criteria (TASK-AGS-705 §Validation Criteria):
//!   (2) groq has GroqNested tool_call_format  -> `groq_has_groq_nested_tool_call_format`
//!   (3) DeepSeek ignore_response_fields includes "logprobs"
//!                                              -> `deepseek_ignores_logprobs_field`
//!       + end-to-end parse succeeds with logprobs in body
//!                                              -> `deepseek_chat_response_with_logprobs_parses_ok`
//!   (4) mistral stream delimiter is NDJSON     -> `mistral_has_ndjson_stream_delimiter`
//!   (5) other 28 compat providers have DEFAULT -> `other_providers_have_default_quirks`
//!   (6) zero string-based provider branching   -> `openai_compat_has_no_string_based_provider_branching`

use std::sync::Arc;

use serde_json::json;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use archon_llm::ApiKey;
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::{
    OPENAI_COMPAT_REGISTRY, OpenAiCompatProvider, ProviderQuirks, StreamDelimiter, ToolCallFormat,
    get_compat,
};

// ---------------------------------------------------------------------------
// Per-provider quirk assignment tests (validation criteria 2-5)
// ---------------------------------------------------------------------------

#[test]
fn groq_has_groq_nested_tool_call_format() {
    let d = OPENAI_COMPAT_REGISTRY
        .get("groq")
        .expect("groq descriptor must exist (TASK-AGS-702)");
    assert_eq!(
        d.quirks.tool_call_format,
        ToolCallFormat::GroqNested,
        "TASK-AGS-705: groq must use GroqNested tool_call_format"
    );
}

#[test]
fn deepseek_ignores_logprobs_field() {
    let d = OPENAI_COMPAT_REGISTRY
        .get("deepseek")
        .expect("deepseek descriptor must exist");
    assert!(
        d.quirks.ignore_response_fields.contains(&"logprobs"),
        "TASK-AGS-705: deepseek must list `logprobs` in ignore_response_fields; got: {:?}",
        d.quirks.ignore_response_fields
    );
}

#[test]
fn mistral_has_ndjson_stream_delimiter() {
    let d = OPENAI_COMPAT_REGISTRY
        .get("mistral")
        .expect("mistral descriptor must exist");
    assert_eq!(
        d.quirks.stream_delimiter,
        StreamDelimiter::MistralNdjson,
        "TASK-AGS-705: mistral must use MistralNdjson stream delimiter"
    );
}

#[test]
fn other_providers_have_default_quirks() {
    let non_default: &[&str] = &["groq", "deepseek", "mistral"];
    let mut checked = 0usize;
    for (id, desc) in OPENAI_COMPAT_REGISTRY.iter() {
        if non_default.contains(id) {
            // These are allowed to deviate.
            continue;
        }
        assert_eq!(
            desc.quirks,
            ProviderQuirks::DEFAULT,
            "TASK-AGS-705: provider `{id}` must have DEFAULT quirks; got: {:?}",
            desc.quirks
        );
        checked += 1;
    }
    // 31 total - 3 non-default = 28 checked.
    assert_eq!(
        checked, 28,
        "TASK-AGS-705: expected to verify 28 default-quirk providers (31 total - 3 non-default)"
    );
}

#[test]
fn default_quirks_constants_match_expected_values() {
    let d = ProviderQuirks::DEFAULT;
    assert_eq!(d.tool_call_format, ToolCallFormat::Standard);
    assert_eq!(d.stream_delimiter, StreamDelimiter::Sse);
    assert!(d.ignore_response_fields.is_empty());

    let g = ProviderQuirks::GROQ;
    assert_eq!(g.tool_call_format, ToolCallFormat::GroqNested);

    let s = ProviderQuirks::DEEPSEEK;
    assert!(s.ignore_response_fields.contains(&"logprobs"));

    let m = ProviderQuirks::MISTRAL;
    assert_eq!(m.stream_delimiter, StreamDelimiter::MistralNdjson);
}

// ---------------------------------------------------------------------------
// End-to-end: DeepSeek-style body with `logprobs` parses cleanly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deepseek_chat_response_with_logprobs_parses_ok() {
    let server = MockServer::start().await;

    // Canned body that mimics DeepSeek: a valid chat.completion plus a
    // top-level `logprobs` blob and a `logprobs` under choices[0].
    let body = json!({
        "id": "chatcmpl-ds-test",
        "object": "chat.completion",
        "created": 0_i64,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hi from deepseek"},
            "finish_reason": "stop",
            "logprobs": {"content": [{"token": "hi", "logprob": -0.1}]}
        }],
        "usage": {"prompt_tokens": 3_u64, "completion_tokens": 4_u64, "total_tokens": 7_u64},
        "logprobs": {"top_level_logprobs": true}
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    // Build a descriptor that mirrors the real deepseek entry but points
    // at the mock server. Quirks are carried over so the impl actually
    // exercises the ignore_response_fields code path.
    let deepseek = OPENAI_COMPAT_REGISTRY
        .get("deepseek")
        .expect("deepseek descriptor must exist");
    let mut mocked = deepseek.clone();
    mocked.base_url = Url::parse(&server.uri()).expect("mock uri parses");
    let leaked: &'static archon_llm::providers::ProviderDescriptor = Box::leak(Box::new(mocked));

    let provider = OpenAiCompatProvider::new(
        leaked,
        Arc::new(reqwest::Client::new()),
        ApiKey::new("test-key".into()),
    );

    let req = LlmRequest {
        model: "deepseek-chat".to_string(),
        max_tokens: 32,
        system: Vec::new(),
        messages: vec![json!({"role": "user", "content": "hi"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        extra: serde_json::Value::Null,
        request_origin: None,
    };

    let resp = provider
        .complete(req)
        .await
        .expect("deepseek body with logprobs must parse successfully");
    assert!(!resp.content.is_empty());
    assert_eq!(
        resp.content[0]["text"].as_str(),
        Some("hi from deepseek"),
        "content must be extracted even when `logprobs` is present"
    );
    assert_eq!(resp.usage.input_tokens, 3);
    assert_eq!(resp.usage.output_tokens, 4);
    assert_eq!(resp.stop_reason, "stop");
}

// ---------------------------------------------------------------------------
// Default-quirks provider still parses a vanilla body (regression guard)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn default_quirks_provider_parses_vanilla_body() {
    let server = MockServer::start().await;
    let body = json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 0_i64,
        "model": "test",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "plain"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1_u64, "completion_tokens": 1_u64, "total_tokens": 2_u64}
    });
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    // Use any DEFAULT-quirks provider, e.g. ollama. Override base_url.
    let ollama = OPENAI_COMPAT_REGISTRY
        .get("ollama")
        .expect("ollama descriptor exists");
    let mut mocked = ollama.clone();
    mocked.base_url = Url::parse(&server.uri()).unwrap();
    let leaked: &'static archon_llm::providers::ProviderDescriptor = Box::leak(Box::new(mocked));

    let provider = OpenAiCompatProvider::new(
        leaked,
        Arc::new(reqwest::Client::new()),
        ApiKey::new("ignored".into()),
    );
    let req = LlmRequest {
        model: "test".into(),
        max_tokens: 16,
        system: Vec::new(),
        messages: vec![json!({"role":"user","content":"hi"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        extra: serde_json::Value::Null,
        request_origin: None,
    };
    let resp = provider.complete(req).await.expect("default quirks path");
    assert_eq!(resp.content[0]["text"].as_str(), Some("plain"));
}

// ---------------------------------------------------------------------------
// get_compat() returns descriptors with quirks intact
// ---------------------------------------------------------------------------

#[test]
fn get_compat_exposes_quirks() {
    let groq = get_compat("groq").expect("groq exists");
    assert_eq!(groq.quirks.tool_call_format, ToolCallFormat::GroqNested);
    let missing = get_compat("notarealprovider");
    assert!(missing.is_none());
}

// ---------------------------------------------------------------------------
// REQ-FOR-D6 core contract: ZERO string-based provider branching in
// openai_compat.rs. This is the invariant that proves quirks-dispatch.
// ---------------------------------------------------------------------------

#[test]
fn openai_compat_has_no_string_based_provider_branching() {
    // Locate the source file at compile time via CARGO_MANIFEST_DIR.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src = format!("{manifest_dir}/src/providers/openai_compat.rs");
    let content = std::fs::read_to_string(&src).unwrap_or_else(|e| panic!("must read {src}: {e}"));

    // Strip comments and string literals inside doc blocks are not
    // branching logic. But a simple substring scan for `== "groq"` etc.
    // is exactly the grep the spec invokes (§Test Commands).
    let forbidden = [
        "== \"groq\"",
        "== \"deepseek\"",
        "== \"mistral\"",
        "id == \"groq\"",
        "id == \"deepseek\"",
        "id == \"mistral\"",
    ];
    for pat in forbidden {
        assert!(
            !content.contains(pat),
            "TASK-AGS-705 REQ-FOR-D6 violation: openai_compat.rs must not branch on \
             provider id (`{pat}`). Use descriptor.quirks instead."
        );
    }
}
