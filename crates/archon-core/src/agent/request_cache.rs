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
pub(crate) fn resolve_strategy(provider: &dyn LlmProvider, configured: &str) -> CacheStrategy {
    let configured = configured.trim();
    if configured.is_empty() || configured.eq_ignore_ascii_case("auto") {
        return provider.cache_strategy();
    }
    match cache_strategy::parse_override(configured) {
        Some(strategy) => strategy,
        None => {
            tracing::warn!(
                "unknown prompt_cache_strategy {configured:?}; falling back to the provider's own \
                 capability. Valid: auto, anthropic, bedrock, responses, automatic, off"
            );
            provider.cache_strategy()
        }
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
    configured: &str,
    enabled: bool,
    mode: &str,
    ttl: &str,
) {
    let strategy = resolve_strategy(provider, configured);
    apply_system_cache_with(request, strategy, enabled, mode, ttl);
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

pub(crate) fn apply_conversation_cache(
    request: &mut LlmRequest,
    provider: &dyn LlmProvider,
    configured: &str,
    enabled: bool,
    mode: &str,
    ttl: &str,
) {
    let strategy = resolve_strategy(provider, configured);
    apply_conversation_cache_with(request, strategy, enabled, mode, ttl);
}

pub(crate) fn apply_conversation_cache_with(
    request: &mut LlmRequest,
    strategy: CacheStrategy,
    enabled: bool,
    mode: &str,
    ttl: &str,
) {
    if !matches!(strategy, CacheStrategy::AnthropicBreakpoints { .. }) {
        // Strips inherited markers as well as declining to add one. A request
        // carrying `cache_control` to an endpoint that does not accept it is a
        // 400 on every turn, not a missed saving.
        remove_cache_directives(request);
        return;
    }
    if !should_emit(strategy, enabled, mode) {
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
