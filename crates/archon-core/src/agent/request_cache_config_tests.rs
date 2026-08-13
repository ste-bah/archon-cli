//! Config-authority tests for `request_cache.rs`: the Bedrock directive and the
//! `[context.prompt_cache_models]` override path.
//!
//! Split from `request_cache_tests.rs` for the 500-line gate. These exercise
//! the FULL signatures — the override map is the subject here, where the other
//! file pins marker placement and shims it empty.

use std::collections::BTreeMap;

use super::{CacheStrategy, apply_conversation_cache, cache_strategy, resolve_strategy};
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::AuthProvider;
use archon_llm::cache_models::{CachePlatform, ModelCacheParams};
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::providers::AnthropicProvider;
use archon_llm::streaming::StreamEvent;
use archon_llm::types::Secret;

fn official_anthropic() -> AnthropicProvider {
    let identity = IdentityProvider::new(
        IdentityMode::Clean,
        "session".into(),
        "device".into(),
        String::new(),
    );
    AnthropicProvider::new(AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("test-key".into())),
        identity,
        None,
    ))
}

/// Mirrors the real Bedrock provider: a Converse checkpoint strategy carrying
/// AWS's Sonnet 4.5 figures, on the Bedrock platform.
struct BedrockLikeProvider;

#[async_trait::async_trait]
impl LlmProvider for BedrockLikeProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        feature == ProviderFeature::PromptCaching
    }

    fn cache_strategy(&self, _model: &str) -> CacheStrategy {
        CacheStrategy::BedrockCachePoint {
            max: 4,
            min_tokens: 4096,
            ttl_1h: true,
        }
    }

    fn cache_platform(&self) -> CachePlatform {
        CachePlatform::Bedrock
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
            serde_json::json!({"role":"user","content":[{"type":"text","text":"latest"}]}),
        ],
        ..LlmRequest::default()
    }
}

fn bedrock_directive(request: &LlmRequest) -> Option<&serde_json::Value> {
    request
        .extra
        .get(cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY)
}

// ---------------------------------------------------------------------------
// The Bedrock directive: config authority over the wire layer
// ---------------------------------------------------------------------------

/// Converse checkpoints are emitted inside the provider, which cannot see
/// config. This directive is how the operator's decision reaches it — and
/// before it existed, the provider emitted checkpoints with `prompt_cache =
/// false` and requested one-hour retention against a five-minute config.
#[test]
fn an_enabled_bedrock_strategy_attaches_the_resolved_directive() {
    let mut request = request();

    apply_conversation_cache(
        &mut request,
        &BedrockLikeProvider,
        "auto",
        true,
        true,
        "explicit",
        "5m",
        &BTreeMap::new(),
    );

    let directive = bedrock_directive(&request).expect("directive must be attached");
    assert_eq!(directive["max"], 4);
    assert_eq!(directive["min_tokens"], 4096);
    assert_eq!(
        directive["ttl_1h"], false,
        "the model supports an hour but the operator asked for five minutes"
    );
}

#[test]
fn the_directive_requests_an_hour_only_when_config_and_model_agree() {
    let mut request = request();

    apply_conversation_cache(
        &mut request,
        &BedrockLikeProvider,
        "auto",
        true,
        true,
        "explicit",
        "1h",
        &BTreeMap::new(),
    );

    assert_eq!(
        bedrock_directive(&request).expect("directive")["ttl_1h"],
        true
    );
}

/// `prompt_cache = false` must actually mean off. The directive is the only
/// channel the provider listens to, so its absence is what "off" looks like at
/// the wire layer.
#[test]
fn a_disabled_config_attaches_no_directive() {
    for (enabled, mode) in [(false, "explicit"), (true, "automatic")] {
        let mut request = request();
        // Simulate a stale directive inherited from a cloned request; disabling
        // must remove it, not merely decline to add one.
        request.extra = serde_json::json!({
            cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY: {"max": 4, "min_tokens": 1024}
        });

        apply_conversation_cache(
            &mut request,
            &BedrockLikeProvider,
            "auto",
            enabled,
            true,
            mode,
            "5m",
            &BTreeMap::new(),
        );

        assert_eq!(
            bedrock_directive(&request),
            None,
            "enabled={enabled} mode={mode}: the directive must be gone"
        );
    }
}

// ---------------------------------------------------------------------------
// [context.prompt_cache_models]: the operator's numbers reach the strategy
// ---------------------------------------------------------------------------

/// The whole reason the table is configurable: a model released after the
/// binary, or a figure a vendor revised, is a config edit rather than a
/// release. This pins that the entry actually reaches the resolved strategy —
/// it was once parsed into config and read by nothing.
#[test]
fn a_configured_model_entry_overrides_the_strategy_limits() {
    let mut overrides = BTreeMap::new();
    overrides.insert(
        "claude-sonnet-4-6".to_string(),
        ModelCacheParams {
            min_tokens: 2048,
            max_checkpoints: 2,
            ttl_1h: false,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    );

    let direct = official_anthropic();
    let strategy = resolve_strategy(&direct, "claude-sonnet-4-6", "auto", &overrides);

    assert_eq!(strategy.min_tokens(), 2048, "config must beat the built-in");
    assert_eq!(strategy.max_breakpoints(), 2);
    assert!(!strategy.supports_1h_ttl());
}

/// The override resolves through the same platform logic as the built-ins, so
/// an entry with a `bedrock_min_tokens` split lands with AWS's figure on a
/// Bedrock provider and Anthropic's on the first-party API.
#[test]
fn a_configured_entry_resolves_against_the_providers_platform() {
    let mut overrides = BTreeMap::new();
    overrides.insert(
        "claude-sonnet-4-6".to_string(),
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: Some(8192),
            bedrock_ttl_1h: Some(false),
        },
    );

    let on_bedrock = resolve_strategy(
        &BedrockLikeProvider,
        "claude-sonnet-4-6",
        "auto",
        &overrides,
    );
    assert_eq!(on_bedrock.min_tokens(), 8192);
    assert!(!on_bedrock.supports_1h_ttl());

    let direct = official_anthropic();
    let on_anthropic = resolve_strategy(&direct, "claude-sonnet-4-6", "auto", &overrides);
    assert_eq!(on_anthropic.min_tokens(), 1024);
    assert!(on_anthropic.supports_1h_ttl());
}

/// The configured numbers must survive into the Bedrock directive — config
/// reaching the strategy but not the wire would be the same dead knob with a
/// longer fuse.
#[test]
fn a_configured_entry_reaches_the_bedrock_directive() {
    let mut overrides = BTreeMap::new();
    overrides.insert(
        "claude-sonnet-4-6".to_string(),
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: Some(8192),
            bedrock_ttl_1h: None,
        },
    );

    let mut request = LlmRequest {
        model: "claude-sonnet-4-6".to_string(),
        ..request()
    };
    apply_conversation_cache(
        &mut request,
        &BedrockLikeProvider,
        "auto",
        true,
        true,
        "explicit",
        "5m",
        &overrides,
    );

    assert_eq!(
        bedrock_directive(&request).expect("directive")["min_tokens"],
        8192,
        "the operator's Bedrock figure must be what the wire layer sees"
    );
}

/// A model the config says nothing about keeps the provider's own numbers —
/// the override map extends the table, it does not replace it.
#[test]
fn an_unconfigured_model_keeps_the_providers_numbers() {
    let mut overrides = BTreeMap::new();
    overrides.insert(
        "claude-opus-9".to_string(),
        ModelCacheParams {
            min_tokens: 512,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    );

    let direct = official_anthropic();
    let strategy = resolve_strategy(&direct, "claude-sonnet-4-6", "auto", &overrides);
    assert_eq!(strategy.min_tokens(), 1024, "built-in Sonnet 4.6 figure");
}
