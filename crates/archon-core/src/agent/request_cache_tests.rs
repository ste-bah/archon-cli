//! Tests for `request_cache.rs`, held in their own file to keep that module
//! under the 500-line ceiling.

use std::collections::BTreeMap;

use super::{CacheStrategy, cache_marker, cache_strategy};
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::providers::AnthropicProvider;
use archon_llm::streaming::StreamEvent;
use archon_llm::types::Secret;

// Most tests here are about strategy resolution and marker placement, not the
// config table, so these shims pin the empty-override case once instead of
// threading `&BTreeMap::new()` through every call. The override path has its
// own tests at the bottom of the file, against the full signatures.

fn resolve_strategy(provider: &dyn LlmProvider, model: &str, configured: &str) -> CacheStrategy {
    super::resolve_strategy(provider, model, configured, &BTreeMap::new())
}

fn apply_system_cache(
    request: &mut LlmRequest,
    provider: &dyn LlmProvider,
    configured: &str,
    enabled: bool,
    mode: &str,
    ttl: &str,
) {
    super::apply_system_cache(
        request,
        provider,
        &super::CacheSettings {
            configured,
            enabled,
            mode,
            ttl,
            model_overrides: &BTreeMap::new(),
        },
    );
}

fn apply_conversation_cache(
    request: &mut LlmRequest,
    provider: &dyn LlmProvider,
    configured: &str,
    enabled: bool,
    mode: &str,
    ttl: &str,
) {
    super::apply_conversation_cache(
        request,
        provider,
        // These tests are about the marker itself; declining the conversation
        // checkpoint has its own test in `request_cache_config_tests`.
        true,
        &super::CacheSettings {
            configured,
            enabled,
            mode,
            ttl,
            model_overrides: &BTreeMap::new(),
        },
    );
}

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

struct NativeCachingProvider;

#[async_trait::async_trait]
impl LlmProvider for NativeCachingProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        feature == ProviderFeature::PromptCaching
    }

    /// Mirrors the real Bedrock provider: a Converse checkpoint strategy with
    /// AWS's Sonnet 4.5 figures.
    fn cache_strategy(&self, _model: &str) -> CacheStrategy {
        CacheStrategy::BedrockCachePoint {
            max: 4,
            min_tokens: 4096,
            ttl_1h: true,
        }
    }

    fn cache_platform(&self) -> archon_llm::cache_models::CachePlatform {
        archon_llm::cache_models::CachePlatform::Bedrock
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        unreachable!()
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!()
    }
}

fn request() -> LlmRequest {
    LlmRequest {
        system: vec![serde_json::json!({"type":"text","text":"system"})],
        messages: vec![
            serde_json::json!({"role":"user","content":"first"}),
            serde_json::json!({"role":"assistant","content":[{"type":"text","text":"reply"}]}),
            serde_json::json!({"role":"user","content":[{"type":"text","text":"latest"}]}),
        ],
        tools: archon_llm::provider::shared_tools(vec![
            serde_json::json!({"name":"Read","cache_control":{"type":"ephemeral"}}),
        ]),
        ..LlmRequest::default()
    }
}

#[test]
fn official_anthropic_marks_stable_system_boundary_when_enabled() {
    let direct = provider(None);
    let mut request = request();
    request
        .system
        .push(serde_json::json!({"type":"text","text":"stable workflow universe"}));

    apply_system_cache(&mut request, &direct, "auto", true, "explicit", "5m");

    assert_eq!(request.system[1]["cache_control"]["type"], "ephemeral");
    assert_eq!(request.system[0].get("cache_control"), None);
}

#[test]
fn disabled_or_unsupported_system_cache_removes_markers() {
    let direct = provider(None);
    let proxy = provider(Some("http://localhost:11434/v1/messages"));
    for (provider, enabled, mode) in [
        (&direct as &dyn LlmProvider, false, "explicit"),
        (&direct as &dyn LlmProvider, true, "automatic"),
        (&proxy as &dyn LlmProvider, true, "explicit"),
    ] {
        let mut request = request();
        request.system[0]["cache_control"] = serde_json::json!({"type":"ephemeral"});

        apply_system_cache(&mut request, provider, "auto", enabled, mode, "5m");

        assert_eq!(request.system[0].get("cache_control"), None);
    }
}

#[test]
fn anthropic_marks_latest_conversation_block_without_mutating_source() {
    let original = request();
    let mut projected = original.clone();

    let direct = provider(None);
    apply_conversation_cache(&mut projected, &direct, "auto", true, "explicit", "5m");

    assert_eq!(
        original.messages[2]["content"][0].get("cache_control"),
        None
    );
    assert_eq!(
        projected.messages[2]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(projected.messages[2]["content"][0]["text"], "latest");
}

#[test]
fn anthropic_compatible_proxy_does_not_receive_conversation_marker() {
    let mut request = request();
    request.system[0]["cache_control"] = serde_json::json!({"type":"ephemeral"});
    let proxy = provider(Some("http://localhost:11434/v1/messages"));

    apply_conversation_cache(&mut request, &proxy, "auto", true, "explicit", "5m");

    assert_eq!(request.system[0].get("cache_control"), None);
    assert_eq!(request.messages[2]["content"][0].get("cache_control"), None);
    assert_eq!(request.tools[0].get("cache_control"), None);
}

#[test]
fn native_prompt_caching_provider_does_not_receive_anthropic_marker() {
    let mut request = request();

    apply_conversation_cache(
        &mut request,
        &NativeCachingProvider,
        "auto",
        true,
        "explicit",
        "5m",
    );

    assert_eq!(request.messages[2]["content"][0].get("cache_control"), None);
}

#[test]
fn unsupported_or_disabled_modes_do_not_mark_conversation() {
    let direct = provider(None);
    let proxy = provider(Some("http://localhost:11434/v1/messages"));
    for (provider, enabled, mode) in [
        (&proxy as &dyn LlmProvider, true, "explicit"),
        (&direct as &dyn LlmProvider, false, "explicit"),
        (&direct as &dyn LlmProvider, true, "automatic"),
    ] {
        let mut request = request();
        apply_conversation_cache(&mut request, provider, "auto", enabled, mode, "5m");
        assert_eq!(request.messages[2]["content"][0].get("cache_control"), None);
    }
}

#[test]
fn conversation_marker_is_added_before_anthropic_wire_budgeting() {
    let mut request = request();
    request.system = (0..3)
        .map(|index| {
            serde_json::json!({
                "type":"text",
                "text":format!("system-{index}"),
                "cache_control":{"type":"ephemeral"}
            })
        })
        .collect();

    let direct = provider(None);
    apply_conversation_cache(&mut request, &direct, "auto", true, "explicit", "5m");

    assert_eq!(
        request.messages[2]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
}

#[test]
fn marks_latest_tool_result_block_for_incremental_tool_round_caching() {
    let mut request = request();
    request.messages.push(serde_json::json!({
        "role":"user",
        "content":[{
            "type":"tool_result",
            "tool_use_id":"tool-1",
            "content":"result"
        }]
    }));

    let direct = provider(None);
    apply_conversation_cache(&mut request, &direct, "auto", true, "explicit", "5m");

    assert_eq!(
        request.messages[3]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(request.messages[2]["content"][0].get("cache_control"), None);
}

#[test]
fn skips_contentless_messages_when_finding_latest_cacheable_block() {
    let mut request = request();
    request
        .messages
        .push(serde_json::json!({"role":"assistant"}));

    let direct = provider(None);
    apply_conversation_cache(&mut request, &direct, "auto", true, "explicit", "5m");

    assert_eq!(
        request.messages[2]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
}

/// The regression this whole change exists for.
///
/// A gateway URL is not the official endpoint, so `auto` leaves caching off
/// and every request is billed in full. Declaring the endpoint restores it.
#[test]
fn declaring_a_gateway_anthropic_restores_caching_that_auto_leaves_off() {
    let proxy = provider(Some("http://127.0.0.1:1234/v1/messages"));

    let mut under_auto = request();
    apply_conversation_cache(&mut under_auto, &proxy, "auto", true, "explicit", "5m");
    assert_eq!(
        under_auto.messages[2]["content"][0].get("cache_control"),
        None,
        "auto cannot know what a gateway forwards, so it stays off"
    );

    let mut declared = request();
    apply_conversation_cache(&mut declared, &proxy, "anthropic", true, "explicit", "5m");
    assert_eq!(
        declared.messages[2]["content"][0]["cache_control"]["type"], "ephemeral",
        "declaring the endpoint is what turns caching on for a gateway"
    );
}

#[test]
fn declaring_a_gateway_also_restores_the_system_boundary() {
    let proxy = provider(Some("http://127.0.0.1:1234/v1/messages"));
    let mut request = request();
    request
        .system
        .push(serde_json::json!({"type":"text","text":"stable"}));

    apply_system_cache(&mut request, &proxy, "anthropic", true, "explicit", "5m");

    assert_eq!(request.system[1]["cache_control"]["type"], "ephemeral");
}

/// `off` must reproduce the pre-change behaviour exactly, so a deployment
/// has a way back that does not depend on reverting a build.
#[test]
fn off_strips_markers_even_on_the_official_endpoint() {
    let direct = provider(None);
    let mut request = request();
    request.system[0]["cache_control"] = serde_json::json!({"type":"ephemeral"});

    apply_conversation_cache(&mut request, &direct, "off", true, "explicit", "5m");

    assert_eq!(request.system[0].get("cache_control"), None);
    assert_eq!(request.messages[2]["content"][0].get("cache_control"), None);
    assert_eq!(request.tools[0].get("cache_control"), None);
}

/// OpenAI caches a stable prefix with no annotation. Emitting `cache_control`
/// there would be an unsupported field, not a harmless hint — so `automatic`
/// must write nothing while still counting as a caching provider.
#[test]
fn automatic_writes_no_markers() {
    let direct = provider(None);
    let mut request = request();

    apply_conversation_cache(&mut request, &direct, "automatic", true, "explicit", "5m");

    assert_eq!(request.messages[2]["content"][0].get("cache_control"), None);
    assert!(
        cache_strategy::parse_override(
            "automatic",
            "claude-sonnet-4-6",
            archon_llm::cache_models::CachePlatform::AnthropicApi,
        )
        .unwrap()
        .caches()
    );
}

/// A typo must not silently change what a deployment spends in either
/// direction, so it falls back to the provider rather than to a default.
#[test]
fn an_unknown_strategy_falls_back_to_the_provider() {
    let direct = provider(None);
    let proxy = provider(Some("http://127.0.0.1:1234/v1/messages"));

    const MODEL: &str = "claude-sonnet-4-6";

    assert_eq!(
        resolve_strategy(&direct, MODEL, "enabled"),
        direct.cache_strategy(MODEL)
    );
    assert_eq!(
        resolve_strategy(&proxy, MODEL, "enabled"),
        CacheStrategy::None
    );
    // Empty is treated as unset, not as "off".
    assert_eq!(
        resolve_strategy(&direct, MODEL, ""),
        direct.cache_strategy(MODEL)
    );
}

/// The minimum comes from the model, not from the wire format. Declaring an
/// endpoint "anthropic" says how to phrase a breakpoint; it says nothing about
/// how large the prefix must be before one takes effect, and that ranges from
/// 512 to 4,096 across Claude models with no relationship to the version
/// number — Opus 4.5 needs 4,096 while Opus 5 needs 512.
#[test]
fn the_declared_strategy_still_takes_its_limits_from_the_model() {
    let proxy = provider(Some("http://127.0.0.1:1234/v1/messages"));

    let opus_5 = resolve_strategy(&proxy, "claude-opus-5", "anthropic");
    assert_eq!(opus_5.min_tokens(), 512);

    let opus_4_5 = resolve_strategy(&proxy, "claude-opus-4-5", "anthropic");
    assert_eq!(opus_4_5.min_tokens(), 4096);

    let sonnet_4_6 = resolve_strategy(&proxy, "claude-sonnet-4-6", "anthropic");
    assert_eq!(sonnet_4_6.min_tokens(), 1024);
}

/// `1h` is rejected by models that do not support it, so the strategy's own
/// capability decides rather than the configured preference.
#[test]
fn a_one_hour_ttl_is_dropped_for_a_strategy_that_cannot_take_it() {
    let responses = CacheStrategy::ResponsesBreakpoints {
        max: 4,
        min_tokens: 1024,
    };
    assert_eq!(cache_marker(responses, "1h").get("ttl"), None);
    assert_eq!(
        cache_marker(cache_strategy::ANTHROPIC_API, "1h")["ttl"],
        "1h"
    );
}

#[test]
fn one_hour_ttl_is_applied_to_conversation_marker() {
    let mut request = request();

    let direct = provider(None);
    apply_conversation_cache(&mut request, &direct, "auto", true, "hybrid", "1h");

    assert_eq!(
        request.messages[2]["content"][0]["cache_control"]["ttl"],
        "1h"
    );
}
