use std::collections::BTreeMap;

use archon_llm::cache_models::{ModelCacheParams, ModelCacheTable};
use archon_llm::cache_strategy::{self, CacheStrategy};
use archon_llm::provider::{LlmProvider, LlmRequest};

/// The strategy actually in force: the configured override if there is one,
/// otherwise whatever the provider reports.
///
/// The override exists because a URL is not evidence. Archon recognises the
/// official Anthropic Messages endpoint and treats everything else as
/// incapable, which is safe against a gateway that would reject `cache_control`
/// and ruinous against one that would have honoured it — the case that billed a
/// Bedrock deployment for every token of every turn. Only the operator knows
/// which of the two their endpoint is, so only they can say.
///
/// An unrecognised value falls back to the provider rather than guessing. A
/// typo that silently enabled or disabled caching would change what a
/// deployment spends without anything reporting it.
///
/// The override supplies the wire format only. The *limits* still come from the
/// model and from the provider's own stack, because those are not the operator's
/// to choose: declaring a gateway `anthropic` says how to phrase a breakpoint,
/// not how many tokens the service behind it requires before honouring one.
pub(crate) fn resolve_strategy(
    provider: &dyn LlmProvider,
    model: &str,
    configured: &str,
    model_overrides: &BTreeMap<String, ModelCacheParams>,
) -> CacheStrategy {
    let configured = configured.trim();
    let strategy = if configured.is_empty() || configured.eq_ignore_ascii_case("auto") {
        provider.cache_strategy(model)
    } else {
        match cache_strategy::parse_override(configured, model, provider.cache_platform()) {
            Some(strategy) => strategy,
            None => {
                tracing::warn!(
                    "unknown prompt_cache_strategy {configured:?}; falling back to the provider's \
                     own capability. Valid: auto, anthropic, bedrock, responses, automatic, off"
                );
                provider.cache_strategy(model)
            }
        }
    };
    apply_model_overrides(strategy, model, provider, model_overrides)
}

/// Replace the strategy's limits with a `[context.prompt_cache_models]` entry,
/// where one matches.
///
/// This is the choke point that makes the config knob real. The providers
/// consult the compiled-in table, which is right until it is stale — a model
/// released after the binary, or a figure a vendor revised. Applying the
/// operator's entry here, after whichever path produced the strategy, means one
/// config edit corrects every provider and both the `auto` and declared-format
/// paths, without a release. The entry is written against the first-party
/// figures plus optional `bedrock_*` splits, and resolves through the same
/// platform logic as the built-ins, so a gateway still gets the strictest
/// reading of the operator's own numbers.
fn apply_model_overrides(
    strategy: CacheStrategy,
    model: &str,
    provider: &dyn LlmProvider,
    model_overrides: &BTreeMap<String, ModelCacheParams>,
) -> CacheStrategy {
    if model_overrides.is_empty() {
        return strategy;
    }
    match ModelCacheTable::from_config(model_overrides.clone()).lookup_configured(model) {
        Some(params) => strategy.with_limits(params.on(provider.cache_platform())),
        None => strategy,
    }
}

/// Whether markers should be written for this request.
///
/// `Automatic` returns false and that is correct, not an oversight: OpenAI
/// caches a stable prefix with no annotation, so a marker would be an
/// unsupported field rather than a hint.
fn should_emit(strategy: CacheStrategy, enabled: bool, mode: &str) -> bool {
    strategy.emits_breakpoints() && enabled && matches!(mode, "explicit" | "hybrid")
}

pub(crate) fn apply_system_cache(
    request: &mut LlmRequest,
    provider: &dyn LlmProvider,
    settings: &CacheSettings<'_>,
) {
    let strategy = resolve_strategy(
        provider,
        &request.model,
        settings.configured,
        settings.model_overrides,
    );
    apply_system_cache_with(
        request,
        strategy,
        settings.enabled,
        settings.mode,
        settings.ttl,
    );
}

pub(crate) fn apply_system_cache_with(
    request: &mut LlmRequest,
    strategy: CacheStrategy,
    enabled: bool,
    mode: &str,
    ttl: &str,
) {
    // Only Anthropic-shaped endpoints take `cache_control` on a system block.
    // Bedrock Converse wants a separate `cachePoint` element and is handled at
    // the wire layer, so its markers are not written here.
    let anthropic_shaped = matches!(strategy, CacheStrategy::AnthropicBreakpoints { .. });
    if !anthropic_shaped || !should_emit(strategy, enabled, mode) {
        for block in &mut request.system {
            if let Some(object) = block.as_object_mut() {
                object.remove("cache_control");
            }
        }
        return;
    }
    let Some(block) = request
        .system
        .last_mut()
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    block.insert("cache_control".into(), cache_marker(strategy, ttl));
}

/// Cache the stable head of the system prompt, ahead of anything injected per
/// turn.
///
/// `stable_blocks` is how many leading `system` entries came from configuration
/// and are identical on every turn. Everything after them — recalled memories,
/// the inner voice, per-turn reminders — is rebuilt each turn and routinely
/// differs.
///
/// Without this there is one breakpoint, at the end of the conversation, and a
/// prefix is only a hit up to its first changed byte. The volatile blocks sit
/// *in front of* the entire message history, so a single recalled memory
/// changing invalidates every turn behind it: the whole conversation is rewritten
/// each round, and on Bedrock a cache write bills at 1.25x, which makes a
/// perpetually-missing cache cost more than none at all.
///
/// A second breakpoint here bounds that. The tools and the static system prompt
/// — the largest genuinely fixed part of the request — keep hitting whatever
/// churns behind them. It costs one of the four available checkpoints and moves
/// no content, so nothing about what the model sees changes.
pub(crate) fn apply_stable_system_cache(
    request: &mut LlmRequest,
    provider: &dyn LlmProvider,
    stable_blocks: usize,
    settings: &CacheSettings<'_>,
) {
    let strategy = resolve_strategy(
        provider,
        &request.model,
        settings.configured,
        settings.model_overrides,
    );
    apply_stable_system_cache_with(
        request,
        strategy,
        stable_blocks,
        settings.enabled,
        settings.mode,
        settings.ttl,
    );
}

/// The `[context]` prompt-cache settings, borrowed as one thing.
///
/// The five of them travelled together through every entry point below and
/// always come from the same config section. Passing them individually made
/// both entry points eight-argument functions whose signatures said nothing
/// about which parameters belonged together — and made it possible to pass
/// `mode` where `ttl` was meant, since both are `&str`.
///
/// Every call site builds it the same way, from `[context]`, so
/// [`CacheSettings::from_context`] is the only constructor anyone needs.
pub(crate) struct CacheSettings<'a> {
    /// `prompt_cache_strategy` — the configured strategy name.
    pub configured: &'a str,
    /// `prompt_cache` — the master switch.
    pub enabled: bool,
    pub mode: &'a str,
    pub ttl: &'a str,
    pub model_overrides: &'a BTreeMap<String, ModelCacheParams>,
}

impl<'a> CacheSettings<'a> {
    /// Borrow the five prompt-cache settings out of `[context]`.
    pub(crate) fn from_context(context: &'a crate::config::ContextConfig) -> Self {
        Self {
            configured: &context.prompt_cache_strategy,
            enabled: context.prompt_cache,
            mode: &context.prompt_cache_mode,
            ttl: &context.prompt_cache_ttl,
            model_overrides: &context.prompt_cache_models,
        }
    }
}

pub(crate) fn apply_stable_system_cache_with(
    request: &mut LlmRequest,
    strategy: CacheStrategy,
    stable_blocks: usize,
    enabled: bool,
    mode: &str,
    ttl: &str,
) {
    // Record the boundary for providers that place their own checkpoints at the
    // wire layer. Bedrock writes `cachePoint` as a separate array element inside
    // the provider and cannot see this call, but it needs the same answer: put
    // the system checkpoint after the stable head, not behind the per-turn
    // content. Recorded before the Anthropic-only early return below, and under
    // its own key so `apply_conversation_cache` cannot clobber it.
    if should_emit(strategy, enabled, mode) && stable_blocks > 0 {
        if !request.extra.is_object() {
            request.extra = serde_json::json!({});
        }
        request.extra[cache_strategy::STABLE_SYSTEM_BLOCKS_KEY] = serde_json::json!(stable_blocks);
    }

    if !matches!(strategy, CacheStrategy::AnthropicBreakpoints { .. })
        || !should_emit(strategy, enabled, mode)
    {
        return;
    }
    // Nothing volatile follows, so the conversation breakpoint already covers
    // this exact prefix and a second one here would be spent for nothing.
    if stable_blocks == 0 || stable_blocks >= request.system.len() {
        return;
    }
    let marker = cache_marker(strategy, ttl);
    if let Some(block) = request
        .system
        .get_mut(stable_blocks - 1)
        .and_then(serde_json::Value::as_object_mut)
    {
        block.insert("cache_control".into(), marker);
    }
}

/// Place the conversation breakpoint, and — for the providers that build their
/// checkpoints at the wire layer — attach the directive that authorises them.
///
/// `enabled` is `prompt_cache`; `conversation` is `prompt_cache_conversation`.
/// They are separate parameters rather than one `&&` because the second only
/// ever meant "do not spend a checkpoint on the message history". Folding them
/// together disabled the *whole* directive on Bedrock and OpenAI, so an operator
/// who declined message caching silently lost the tools and system checkpoints
/// as well — the expensive ones, and the ones they had not asked to give up.
pub(crate) fn apply_conversation_cache(
    request: &mut LlmRequest,
    provider: &dyn LlmProvider,
    conversation: bool,
    settings: &CacheSettings<'_>,
) {
    let strategy = resolve_strategy(
        provider,
        &request.model,
        settings.configured,
        settings.model_overrides,
    );
    apply_conversation_cache_with(
        request,
        strategy,
        settings.enabled,
        conversation,
        settings.mode,
        settings.ttl,
    );
}

pub(crate) fn apply_conversation_cache_with(
    request: &mut LlmRequest,
    strategy: CacheStrategy,
    enabled: bool,
    conversation: bool,
    mode: &str,
    ttl: &str,
) {
    if let CacheStrategy::BedrockCachePoint {
        max,
        min_tokens,
        ttl_1h,
    } = strategy
    {
        // Bedrock's checkpoints are separate array elements built at the wire
        // layer, so nothing is marked here — instead the resolved, authorised
        // decision travels on the request for the provider to execute. Config
        // is weighed HERE, where it exists: without this, the provider emitted
        // checkpoints with `prompt_cache = false`, and requested one-hour
        // retention — the expensive write tier — while the operator had asked
        // for five minutes.
        // Carried across from `apply_stable_system_cache`, which knows the
        // boundary but runs earlier.
        let stable_blocks = request
            .extra
            .get(cache_strategy::STABLE_SYSTEM_BLOCKS_KEY)
            .and_then(serde_json::Value::as_u64);

        remove_cache_directives(request);
        if should_emit(strategy, enabled, mode) {
            if !request.extra.is_object() {
                request.extra = serde_json::json!({});
            }
            let mut directive = serde_json::json!({
                "max": max,
                "min_tokens": min_tokens,
                "ttl_1h": ttl_1h && ttl == "1h",
                // The tools and system checkpoints stand whatever this says;
                // only the messages one is the operator's to decline.
                "conversation": conversation,
            });
            if let Some(blocks) = stable_blocks {
                directive["stable_system_blocks"] = serde_json::json!(blocks);
            }
            request.extra[cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY] = directive;
        } else if let Some(extra) = request.extra.as_object_mut() {
            extra.remove(cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY);
        }
        return;
    }
    if let CacheStrategy::ResponsesBreakpoints { min_tokens, .. } = strategy {
        // Same split as Bedrock: the marker is a content part built inside the
        // provider, but whether to build one depends on config the provider
        // cannot see. `explicit_only` is the one that matters most — it turns
        // OpenAI's implicit breakpoints off, so sending it on the operator's
        // behalf without being asked could remove caching that was happening for
        // free.
        let stable_blocks = request
            .extra
            .get(cache_strategy::STABLE_SYSTEM_BLOCKS_KEY)
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as usize);

        remove_cache_directives(request);
        if should_emit(strategy, enabled, mode) {
            if !request.extra.is_object() {
                request.extra = serde_json::json!({});
            }
            let mut directive = serde_json::json!({
                "min_tokens": min_tokens,
                "explicit_only": mode == "explicit",
                "cache_key": archon_llm::cache_wire::prompt_cache_key(&request.system, stable_blocks),
            });
            if let Some(blocks) = stable_blocks {
                directive["stable_system_blocks"] = serde_json::json!(blocks);
            }
            request.extra[archon_llm::cache_wire::OPENAI_CACHE_DIRECTIVE_KEY] = directive;
        } else if let Some(extra) = request.extra.as_object_mut() {
            extra.remove(archon_llm::cache_wire::OPENAI_CACHE_DIRECTIVE_KEY);
        }
        return;
    }
    if !matches!(strategy, CacheStrategy::AnthropicBreakpoints { .. }) {
        // Strips inherited markers as well as declining to add one. A request
        // carrying `cache_control` to an endpoint that does not accept it is a
        // 400 on every turn, not a missed saving.
        remove_cache_directives(request);
        return;
    }
    if !should_emit(strategy, enabled && conversation, mode) {
        return;
    }
    let Some(block) = latest_cacheable_block(&mut request.messages) else {
        return;
    };
    block.insert("cache_control".into(), cache_marker(strategy, ttl));
}

/// The `cache_control` value for a breakpoint.
///
/// A one-hour TTL is requested only where the model supports it. Sending `1h`
/// to a model that does not is rejected outright, so the strategy's own
/// capability decides rather than the configured preference.
fn cache_marker(strategy: CacheStrategy, ttl: &str) -> serde_json::Value {
    let mut marker = serde_json::json!({"type": "ephemeral"});
    if ttl == "1h" && strategy.supports_1h_ttl() {
        marker["ttl"] = serde_json::json!("1h");
    }
    marker
}

fn remove_cache_directives(request: &mut LlmRequest) {
    // #171 part 3: `tools` is a shared frozen list, so only take the
    // copy-on-write path when a marker is actually present. Tool schemas
    // built from the registry never carry one, which is why the shared list
    // survives untouched on every non-Anthropic round.
    if request
        .tools
        .iter()
        .any(|tool| tool.get("cache_control").is_some())
    {
        for tool in std::sync::Arc::make_mut(&mut request.tools) {
            if let Some(object) = tool.as_object_mut() {
                object.remove("cache_control");
            }
        }
    }
    for block in &mut request.system {
        if let Some(object) = block.as_object_mut() {
            object.remove("cache_control");
        }
    }
    for message in &mut request.messages {
        let Some(blocks) = message
            .get_mut("content")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for block in blocks {
            if let Some(object) = block.as_object_mut() {
                object.remove("cache_control");
            }
        }
    }
}

fn latest_cacheable_block(
    messages: &mut [serde_json::Value],
) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    for message in messages.iter_mut().rev() {
        let Some(content) = message.get_mut("content") else {
            continue;
        };
        match content {
            serde_json::Value::Array(blocks) => {
                for block in blocks.iter_mut().rev() {
                    let Some(object) = block.as_object_mut() else {
                        continue;
                    };
                    if matches!(
                        object.get("type").and_then(|value| value.as_str()),
                        Some("text" | "tool_result")
                    ) && !object.contains_key("cache_control")
                    {
                        return Some(object);
                    }
                }
            }
            serde_json::Value::String(text) => {
                let text = std::mem::take(text);
                *content = serde_json::json!([{"type":"text","text":text}]);
                return content.as_array_mut()?.last_mut()?.as_object_mut();
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
#[path = "request_cache_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "request_cache_config_tests.rs"]
mod config_tests;

#[cfg(test)]
#[path = "request_cache_prefix_tests.rs"]
mod prefix_tests;

#[cfg(test)]
#[path = "request_cache_openai_tests.rs"]
mod openai_tests;
