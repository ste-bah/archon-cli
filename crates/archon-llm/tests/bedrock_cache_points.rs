//! Bedrock Converse prompt-cache checkpoints.
//!
//! Converse does not take `cache_control` as an attribute on a content block the
//! way the Anthropic Messages API does. It takes a checkpoint as its **own array
//! element**:
//!
//! ```json
//! { "cachePoint": { "type": "default" } }
//! ```
//!
//! That shape difference is why Bedrock has its own `CacheStrategy` variant
//! rather than reusing the Anthropic one, and these tests pin it — an attribute
//! written where an element belongs is silently ignored, so the request would
//! succeed, be billed in full, and look exactly like a cache hit.

use std::sync::Arc;

use archon_llm::cache_models::CachePlatform;
use archon_llm::cache_strategy::{BEDROCK_CACHE_DIRECTIVE_KEY, CachePointPlacement, CacheStrategy};
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::BedrockProvider;

fn provider(model_id: &str) -> BedrockProvider {
    BedrockProvider::new("eu-west-2".to_string(), model_id.to_string())
}

/// The resolved decision `archon-core` attaches after weighing the operator's
/// config. The provider acts only on this — never on the model table directly.
fn directive(min_tokens: usize, ttl_1h: bool) -> serde_json::Value {
    serde_json::json!({
        BEDROCK_CACHE_DIRECTIVE_KEY: {
            "max": 4,
            "min_tokens": min_tokens,
            "ttl_1h": ttl_1h,
        }
    })
}

/// Enough text to clear any model's minimum without asserting on a token count.
/// The gate uses a four-characters-per-token estimate, so this is comfortably
/// past 4,096 tokens.
fn bulky_request(model: &str) -> LlmRequest {
    LlmRequest {
        model: model.to_string(),
        system: vec![serde_json::json!({"type": "text", "text": "s".repeat(40_000)})],
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hello"}]
        })],
        tools: Arc::new(vec![serde_json::json!({
            "name": "read",
            "description": "d".repeat(4_000),
            "input_schema": {"type": "object"}
        })]),
        extra: directive(4096, false),
        ..LlmRequest::default()
    }
}

fn cache_points(section: &serde_json::Value) -> usize {
    section
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("cachePoint").is_some())
                .count()
        })
        .unwrap_or(0)
}

/// A checkpoint is its own element, appended after the content it caches — not
/// a key on the neighbouring block.
#[test]
fn a_checkpoint_is_its_own_array_element_in_every_section() {
    let system = vec![serde_json::json!({"type": "text", "text": "you are archon"})];
    let messages = vec![serde_json::json!({
        "role": "user",
        "content": [{"type": "text", "text": "hi"}]
    })];
    let tools = vec![serde_json::json!({
        "name": "read",
        "description": "read a file",
        "input_schema": {"type": "object"}
    })];

    let body = BedrockProvider::build_converse_body_cached(
        &system,
        &messages,
        &tools,
        8192,
        Some(CachePointPlacement::all(false)),
    );

    assert_eq!(cache_points(&body["system"]), 1, "system");
    assert_eq!(cache_points(&body["toolConfig"]["tools"]), 1, "tools");
    assert_eq!(
        cache_points(&body["messages"][0]["content"]),
        1,
        "last message"
    );

    // The element carries only `type`, and never leaks onto a sibling block.
    assert_eq!(body["system"][1]["cachePoint"]["type"], "default");
    assert!(
        body["system"][0].get("cache_control").is_none(),
        "cache_control is the Anthropic spelling and must not appear here"
    );
}

/// Without a placement the body must be byte-identical to the uncached builder.
/// Anything else would mean every existing caller silently changed shape.
#[test]
fn no_placement_leaves_the_body_untouched() {
    let system = vec![serde_json::json!({"type": "text", "text": "you are archon"})];
    let tools = vec![serde_json::json!({
        "name": "read",
        "description": "read a file",
        "input_schema": {"type": "object"}
    })];

    let plain = BedrockProvider::build_converse_body(&system, &[], &tools, 8192);
    let uncached = BedrockProvider::build_converse_body_cached(&system, &[], &tools, 8192, None);

    assert_eq!(plain, uncached);
    assert_eq!(cache_points(&uncached["system"]), 0);
    assert_eq!(cache_points(&uncached["toolConfig"]["tools"]), 0);
}

/// The one-hour TTL rides on the checkpoint element itself.
#[test]
fn the_extended_ttl_travels_on_the_checkpoint() {
    let system = vec![serde_json::json!({"type": "text", "text": "you are archon"})];

    let hour = BedrockProvider::build_converse_body_cached(
        &system,
        &[],
        &[],
        8192,
        Some(CachePointPlacement::all(true)),
    );
    assert_eq!(hour["system"][1]["cachePoint"]["ttl"], "1h");

    let default = BedrockProvider::build_converse_body_cached(
        &system,
        &[],
        &[],
        8192,
        Some(CachePointPlacement::all(false)),
    );
    assert!(
        default["system"][1]["cachePoint"].get("ttl").is_none(),
        "five minutes is the default and is expressed by omission"
    );
}

/// Bedrock is the authority on its own endpoint, so the strategy carries AWS's
/// figures rather than Anthropic's — 4,096 for Sonnet 4.5, not 1,024.
#[test]
fn the_strategy_carries_bedrocks_own_limits() {
    let CacheStrategy::BedrockCachePoint {
        max, min_tokens, ..
    } = provider("anthropic.claude-sonnet-4-5-20250929-v1:0")
        .cache_strategy("anthropic.claude-sonnet-4-5-20250929-v1:0")
    else {
        panic!("Claude on Bedrock must use the Converse checkpoint strategy");
    };

    assert_eq!(
        min_tokens, 4096,
        "AWS documents 4,096 where Anthropic says 1,024"
    );
    assert_eq!(max, 4);
}

/// The strategy resolves on the model the provider actually dials, not the
/// spelling the request happens to carry. Requests routinely say `sonnet`, or
/// a bare `claude-sonnet-4-6` with no `anthropic.` vendor prefix, while the
/// endpoint URL is always built from the configured `model_id` — gating on the
/// request's spelling silently disabled caching for every such request.
#[test]
fn the_strategy_ignores_the_requests_spelling_of_the_model() {
    let provider = provider("eu.anthropic.claude-sonnet-4-5-20250929-v1:0");

    for requested in ["sonnet", "claude-sonnet-4-5", ""] {
        let strategy = provider.cache_strategy(requested);
        assert!(
            matches!(strategy, CacheStrategy::BedrockCachePoint { .. }),
            "request spelling {requested:?} must not disable caching for the \
             configured model"
        );
        assert_eq!(strategy.min_tokens(), 4096);
    }
}

/// Opus 4.6 accepts an hour on the first-party API and five minutes on Bedrock.
/// Asking for an unsupported TTL fails the request outright.
#[test]
fn the_extended_ttl_is_not_claimed_where_aws_withholds_it() {
    let strategy = provider("anthropic.claude-opus-4-6-v1:0").cache_strategy("claude-opus-4-6");
    assert!(!strategy.supports_1h_ttl());
}

/// Bedrock hosts several vendors. Only Claude's checkpoint support is
/// documented here, so anything else gets nothing rather than a field its API
/// may reject on every request.
#[test]
fn a_non_claude_model_gets_no_strategy() {
    let strategy = provider("amazon.titan-text-express-v1").cache_strategy("amazon.titan-text");
    assert_eq!(strategy, CacheStrategy::None);
    assert!(!strategy.emits_breakpoints());
}

#[test]
fn the_platform_is_bedrock() {
    assert_eq!(
        provider("anthropic.claude-sonnet-4-6-v1:0").cache_platform(),
        CachePlatform::Bedrock
    );
}

/// No directive on the request means no checkpoints, however large the prompt.
/// The directive is where the operator's `prompt_cache = false` lives; a
/// provider that decided for itself was emitting checkpoints the config had
/// switched off.
#[test]
fn no_directive_means_no_checkpoints_regardless_of_size() {
    let provider = provider("anthropic.claude-sonnet-4-5-20250929-v1:0");
    let mut request = bulky_request("anthropic.claude-sonnet-4-5-20250929-v1:0");
    request.extra = serde_json::Value::Null;

    assert_eq!(provider.cache_placement(&request), None);
}

/// The TTL is the operator's, not the model's. The directive carries the
/// already-resolved answer — capability AND preference — so a five-minute
/// config never produces a one-hour write, which is the expensive tier.
#[test]
fn the_directive_ttl_is_obeyed_verbatim() {
    let provider = provider("anthropic.claude-sonnet-4-5-20250929-v1:0");

    let mut request = bulky_request("anthropic.claude-sonnet-4-5-20250929-v1:0");
    request.extra = directive(4096, true);
    let hour = provider.cache_placement(&request).expect("placement");
    assert!(hour.ttl_1h);

    request.extra = directive(4096, false);
    let five = provider.cache_placement(&request).expect("placement");
    assert!(!five.ttl_1h);
}

/// A prompt under the model's minimum gets no checkpoints. Emitting one would
/// not fail — Bedrock discards it silently — but it would put fields on the wire
/// that do nothing and report a cache that was never written.
#[test]
fn a_prompt_below_the_minimum_gets_no_checkpoints() {
    let provider = provider("anthropic.claude-sonnet-4-5-20250929-v1:0");
    let small = LlmRequest {
        model: "anthropic.claude-sonnet-4-5-20250929-v1:0".to_string(),
        system: vec![serde_json::json!({"type": "text", "text": "you are archon"})],
        extra: directive(4096, false),
        ..LlmRequest::default()
    };

    assert_eq!(provider.cache_placement(&small), None);
}

/// Past the minimum, all three sections are checkpointed. Bedrock evaluates them
/// in the order `tools` -> `system` -> `messages` and chains them, so the most
/// stable content is cached first and three is one under the limit of four.
#[test]
fn a_prompt_past_the_minimum_checkpoints_every_section() {
    let provider = provider("anthropic.claude-sonnet-4-5-20250929-v1:0");
    let placement = provider
        .cache_placement(&bulky_request("anthropic.claude-sonnet-4-5-20250929-v1:0"))
        .expect("a prompt well past 4,096 tokens must be checkpointed");

    assert!(placement.tools);
    assert!(placement.system);
    assert!(placement.messages);
    assert_eq!(placement.count(), 3, "one under Bedrock's limit of four");
}

/// The minimum is measured across `tools`, `system` and `messages` combined,
/// not per section. A prompt whose sections are individually small but jointly
/// large must still be cached.
#[test]
fn the_minimum_is_cumulative_across_the_three_sections() {
    let provider = provider("anthropic.claude-opus-4-5-20251101-v1:0");
    let model = "anthropic.claude-opus-4-5-20251101-v1:0";

    // ~2,000 estimated tokens per section: none clears 4,096 alone, together
    // they clear it comfortably.
    let chunk = "x".repeat(8_000);
    let request = LlmRequest {
        model: model.to_string(),
        system: vec![serde_json::json!({"type": "text", "text": chunk})],
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "y".repeat(8_000)}]
        })],
        tools: Arc::new(vec![serde_json::json!({
            "name": "read",
            "description": "z".repeat(8_000),
            "input_schema": {"type": "object"}
        })]),
        extra: directive(4096, false),
        ..LlmRequest::default()
    };

    assert!(
        provider.cache_placement(&request).is_some(),
        "sections are summed, so three sub-minimum sections still qualify"
    );
}

/// A model with no checkpoint support gets no placement however large the
/// prompt is.
#[test]
fn a_non_claude_model_gets_no_placement_however_large() {
    let provider = provider("amazon.titan-text-express-v1");
    assert_eq!(
        provider.cache_placement(&bulky_request("amazon.titan-text-express-v1")),
        None
    );
}
