//! Prefix-stability tests for `request_cache.rs`: the stable-head breakpoint.
//!
//! Split from `request_cache_tests.rs` for the 500-line gate.
//!
//! The main agent's system prompt is the configured blocks followed by per-turn
//! injections — recalled memories, the inner voice, reminders. Those sit in
//! front of the whole message history, so with only the conversation breakpoint
//! one changed memory invalidated every turn behind it: the entire history was
//! rewritten to cache each round, and on Bedrock a cache write bills at 1.25x.
//! The stable head therefore gets its own breakpoint, and these tests pin where
//! it lands and when it is withheld.

use std::collections::BTreeMap;

use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::AnthropicProvider;
use archon_llm::types::Secret;

fn provider(api_url: Option<&str>) -> AnthropicProvider {
    let identity = IdentityProvider::new(
        IdentityMode::Clean,
        "session".into(),
        "device".into(),
        String::new(),
    );
    AnthropicProvider::new(AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("test-key".into())),
        identity,
        api_url.map(str::to_string),
    ))
}

fn apply_stable(
    request: &mut LlmRequest,
    provider: &dyn LlmProvider,
    stable_blocks: usize,
    enabled: bool,
) {
    super::apply_stable_system_cache(
        request,
        provider,
        stable_blocks,
        &super::CacheSettings {
            configured: "auto",
            enabled,
            mode: "explicit",
            ttl: "5m",
            model_overrides: &BTreeMap::new(),
        },
    );
}

/// The configured blocks, then the per-turn injections behind them.
fn turn_request(stable: usize, volatile: usize) -> LlmRequest {
    let mut system: Vec<serde_json::Value> = (0..stable)
        .map(|i| serde_json::json!({"type":"text","text":format!("configured {i}")}))
        .collect();
    system.extend(
        (0..volatile)
            .map(|i| serde_json::json!({"type":"text","text":format!("recalled memory {i}")})),
    );
    LlmRequest {
        system,
        messages: vec![
            serde_json::json!({"role":"user","content":[{"type":"text","text":"latest"}]}),
        ],
        ..LlmRequest::default()
    }
}

#[test]
fn the_stable_head_is_cached_ahead_of_the_per_turn_blocks() {
    let direct = provider(None);
    let mut request = turn_request(2, 3);

    apply_stable(&mut request, &direct, 2, true);

    assert_eq!(
        request.system[1]["cache_control"]["type"], "ephemeral",
        "the breakpoint belongs at the end of the configured blocks"
    );
    for index in [0, 2, 3, 4] {
        assert_eq!(
            request.system[index].get("cache_control"),
            None,
            "block {index} must not carry a breakpoint"
        );
    }
}

/// With nothing volatile appended, the conversation breakpoint already covers
/// this exact prefix. Spending a second checkpoint on it would buy nothing, and
/// there are only four.
#[test]
fn no_second_breakpoint_when_nothing_follows_the_stable_head() {
    let direct = provider(None);
    let mut request = turn_request(2, 0);

    apply_stable(&mut request, &direct, 2, true);

    assert!(
        request
            .system
            .iter()
            .all(|block| block.get("cache_control").is_none())
    );
}

/// The two breakpoints must coexist: the stable head, and the conversation. That
/// is the whole point — the head keeps hitting when the volatile middle changes
/// and takes the message history's cache down with it.
#[test]
fn the_stable_head_and_the_conversation_are_cached_independently() {
    let direct = provider(None);
    let mut request = turn_request(2, 3);

    apply_stable(&mut request, &direct, 2, true);
    super::apply_conversation_cache(
        &mut request,
        &direct,
        true,
        &super::CacheSettings {
            configured: "auto",
            enabled: true,
            mode: "explicit",
            ttl: "5m",
            model_overrides: &BTreeMap::new(),
        },
    );

    assert_eq!(request.system[1]["cache_control"]["type"], "ephemeral");
    assert_eq!(
        request.messages[0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
}

/// An endpoint that cannot take `cache_control` must not receive one here
/// either, and disabling the feature must silence the breakpoint.
#[test]
fn the_stable_head_breakpoint_respects_the_provider_and_the_switch() {
    let proxy = provider(Some("http://127.0.0.1:1234/v1/messages"));
    let mut via_proxy = turn_request(2, 3);
    apply_stable(&mut via_proxy, &proxy, 2, true);
    assert_eq!(via_proxy.system[1].get("cache_control"), None);

    let direct = provider(None);
    let mut switched_off = turn_request(2, 3);
    apply_stable(&mut switched_off, &direct, 2, false);
    assert_eq!(switched_off.system[1].get("cache_control"), None);
}
