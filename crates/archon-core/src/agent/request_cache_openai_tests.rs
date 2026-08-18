//! The OpenAI cache directive, and the flag split it exposed.
//!
//! GPT-5.6's `prompt_cache_breakpoint` rides on a content part built inside the
//! provider, so — exactly as with Bedrock Converse — the operator's decision has
//! to travel on the request. These pin that channel.

use std::collections::BTreeMap;

use super::{CacheStrategy, apply_conversation_cache, apply_conversation_cache_with};
use archon_llm::cache_models::CachePlatform;
use archon_llm::cache_wire::OPENAI_CACHE_DIRECTIVE_KEY;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;

/// Mirrors `OpenAiProvider` on a GPT-5.6-class model.
struct ResponsesProvider;

#[async_trait::async_trait]
impl LlmProvider for ResponsesProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        feature == ProviderFeature::PromptCaching
    }

    fn cache_strategy(&self, _model: &str) -> CacheStrategy {
        CacheStrategy::ResponsesBreakpoints {
            max: 4,
            min_tokens: 1024,
        }
    }

    fn cache_platform(&self) -> CachePlatform {
        CachePlatform::OpenAiApi
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
        system: vec![
            serde_json::json!({"type":"text","text":"stable"}),
            serde_json::json!({"type":"text","text":"volatile"}),
        ],
        messages: vec![
            serde_json::json!({"role":"user","content":[{"type":"text","text":"latest"}]}),
        ],
        ..LlmRequest::default()
    }
}

fn directive(request: &LlmRequest) -> Option<&serde_json::Value> {
    request.extra.get(OPENAI_CACHE_DIRECTIVE_KEY)
}

fn apply(request: &mut LlmRequest, enabled: bool, mode: &str) {
    apply_conversation_cache(
        request,
        &ResponsesProvider,
        true,
        &super::CacheSettings {
            configured: "auto",
            enabled,
            mode,
            ttl: "5m",
            model_overrides: &BTreeMap::new(),
        },
    );
}

#[test]
fn an_enabled_responses_strategy_attaches_the_directive() {
    let mut request = request();

    apply(&mut request, true, "explicit");

    let directive = directive(&request).expect("directive must be attached");
    assert_eq!(directive["min_tokens"], 1024);
    assert_eq!(
        directive["explicit_only"], true,
        "explicit mode is what authorises turning OpenAI's implicit \
         breakpoints off"
    );
    assert!(
        directive["cache_key"]
            .as_str()
            .is_some_and(|key| key.starts_with("archon:")),
        "GPT-5.6 needs a prompt_cache_key to match reliably at all"
    );
}

/// `hybrid` must not send `prompt_cache_options`. Archon adding a breakpoint is
/// an improvement; archon *removing* the implicit ones is a decision only the
/// operator can make, because a misjudged placement then costs the caching that
/// was happening unprompted.
#[test]
fn hybrid_mode_does_not_authorise_disabling_the_implicit_breakpoints() {
    let mut request = request();

    apply(&mut request, true, "hybrid");

    assert_eq!(
        directive(&request).expect("directive")["explicit_only"],
        false
    );
}

#[test]
fn a_disabled_config_attaches_no_directive() {
    for (enabled, mode) in [(false, "explicit"), (true, "automatic")] {
        let mut request = request();
        // A stale directive inherited from a cloned request must be removed,
        // not merely left unwritten.
        request.extra = serde_json::json!({
            OPENAI_CACHE_DIRECTIVE_KEY: { "min_tokens": 1024 }
        });

        apply(&mut request, enabled, mode);

        assert_eq!(
            directive(&request),
            None,
            "enabled={enabled} mode={mode}: the directive must be gone"
        );
    }
}

/// The boundary recorded by `apply_stable_system_cache` has to survive into the
/// directive — it is the only thing that stops the breakpoint landing behind the
/// per-turn content, where it would be rewritten every turn.
#[test]
fn the_stable_boundary_is_carried_into_the_directive() {
    let mut request = request();
    super::apply_stable_system_cache_with(
        &mut request,
        CacheStrategy::ResponsesBreakpoints {
            max: 4,
            min_tokens: 1024,
        },
        1,
        true,
        "explicit",
        "5m",
    );

    apply(&mut request, true, "explicit");

    assert_eq!(
        directive(&request).expect("directive")["stable_system_blocks"],
        1
    );
}

/// The regression the flag split exists for. `prompt_cache_conversation = false`
/// means "do not spend a checkpoint on the message history". Folded into one
/// flag it removed the whole directive, so the tools and system checkpoints —
/// the expensive ones — went with it.
#[test]
fn declining_the_conversation_checkpoint_keeps_the_rest() {
    let mut request = request();

    apply_conversation_cache_with(
        &mut request,
        CacheStrategy::ResponsesBreakpoints {
            max: 4,
            min_tokens: 1024,
        },
        true,
        false,
        "explicit",
        "5m",
    );

    assert!(
        directive(&request).is_some(),
        "declining message caching must not disable the system breakpoint"
    );
}

#[test]
fn bedrock_records_the_conversation_choice_rather_than_dropping_everything() {
    let mut request = request();

    apply_conversation_cache_with(
        &mut request,
        CacheStrategy::BedrockCachePoint {
            max: 4,
            min_tokens: 4096,
            ttl_1h: false,
        },
        true,
        false,
        "explicit",
        "5m",
    );

    let directive = request
        .extra
        .get(archon_llm::cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY)
        .expect("the tools and system checkpoints still stand");
    assert_eq!(directive["conversation"], false);
}
