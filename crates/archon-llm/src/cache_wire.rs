//! Wire-layer helpers shared by the providers that place their own cache
//! checkpoints.
//!
//! Bedrock Converse and the OpenAI Chat Completions API both want the checkpoint
//! built inside the provider — Converse because it is a separate array element,
//! OpenAI because the breakpoint rides on a content part that only the body
//! builder constructs. Neither decision is the provider's to *make*, though: the
//! operator's config is weighed in `archon-core`, which attaches a resolved
//! directive to `LlmRequest::extra`. These helpers are the shared half of
//! executing it.

/// A rough token count for the whole prompt, used only to decide whether a
/// checkpoint would clear the model's minimum.
///
/// Four characters per token is the usual English approximation. It is not
/// accurate enough to bill against and does not need to be: the only decision it
/// feeds is whether to emit a checkpoint that the service would otherwise
/// discard, and being wrong near the boundary costs a cache write that would
/// have been skipped, or skips one that would have been kept — never an error.
///
/// All three sections are counted together because both services measure the
/// minimum across `tools`, `system` and `messages` combined, not per section.
pub fn estimated_prompt_tokens(
    system: &[serde_json::Value],
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
) -> usize {
    let chars: usize = system
        .iter()
        .chain(messages.iter())
        .chain(tools.iter())
        .map(|value| value.to_string().len())
        .sum();
    chars / 4
}

/// Key under `LlmRequest::extra` carrying the resolved OpenAI cache directive.
///
/// The counterpart to [`crate::cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY`],
/// and present for the same reason: `prompt_cache_breakpoint` is written at the
/// wire layer, but whether to write one depends on `prompt_cache`,
/// `prompt_cache_mode` and `[context.prompt_cache_models]`, none of which the
/// provider sees. Absent means no, so an SDK caller or a test that skipped
/// resolution gets an unannotated request rather than an unconditional
/// breakpoint.
pub const OPENAI_CACHE_DIRECTIVE_KEY: &str = "prompt_cache_openai";

/// The resolved decision for one OpenAI request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCachePlacement {
    /// How many leading `system` blocks are stable across turns. The breakpoint
    /// closes the content part holding exactly these, so the volatile tail sits
    /// behind it. `None` puts the breakpoint after the whole system prompt.
    pub stable_system_blocks: Option<usize>,
    /// Send `prompt_cache_options: {"mode": "explicit"}`, which switches
    /// OpenAI's own implicit breakpoints **off** so only ours participate.
    ///
    /// This is why archon's `prompt_cache_mode` matters here and not just on
    /// Anthropic: `hybrid` leaves the implicit breakpoints in place and adds
    /// ours, so a misjudged placement costs nothing. `explicit` is strictly
    /// better when the placement is right and strictly worse when it is not.
    pub explicit_only: bool,
    /// `prompt_cache_key`. OpenAI routes on a hash of the leading tokens
    /// *combined with this key*, and documents that GPT-5.6 needs one set to get
    /// reliable matching at all.
    pub cache_key: String,
}

/// A stable `prompt_cache_key` derived from the prefix being cached.
///
/// Deriving it from the content rather than plumbing a session id through has
/// the property the key is actually for: two turns of the same conversation
/// share a key because their stable prefix is identical, and two different
/// agents get different keys because theirs are not. Requests that would collide
/// in the cache route together; requests that would not, do not.
///
/// FNV-1a rather than `DefaultHasher`, whose output is explicitly not stable
/// across Rust releases — a rebuild would silently rotate every key and throw
/// away the caches it was meant to address.
pub fn prompt_cache_key(system: &[serde_json::Value], stable_blocks: Option<usize>) -> String {
    let take = stable_blocks
        .filter(|n| *n > 0 && *n <= system.len())
        .unwrap_or(system.len());

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for block in &system[..take] {
        let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
        for byte in text.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("archon:{hash:016x}")
}

/// Read the resolved directive off a request, if the prompt is large enough to
/// be worth a breakpoint.
///
/// The size gate lives here rather than in `archon-core` because only the wire
/// layer sees the rendered prompt. Below the minimum the breakpoint is discarded
/// silently — and with `explicit_only` set that is worse than doing nothing,
/// because turning the implicit breakpoints off would then leave the request
/// with no caching at all.
pub fn openai_cache_placement(
    extra: &serde_json::Value,
    system: &[serde_json::Value],
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
) -> Option<OpenAiCachePlacement> {
    let directive = extra.get(OPENAI_CACHE_DIRECTIVE_KEY)?;
    let min_tokens = directive.get("min_tokens")?.as_u64()? as usize;

    if estimated_prompt_tokens(system, messages, tools) < min_tokens {
        return None;
    }

    let stable_system_blocks = directive
        .get("stable_system_blocks")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as usize);

    Some(OpenAiCachePlacement {
        stable_system_blocks,
        explicit_only: directive
            .get("explicit_only")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        cache_key: directive
            .get("cache_key")
            .and_then(|k| k.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| prompt_cache_key(system, stable_system_blocks)),
    })
}

/// The breakpoint marker itself.
///
/// `explicit` is the only accepted mode. There is deliberately no TTL here:
/// `prompt_cache_options.ttl` accepts `30m` and nothing else, which is also the
/// default, so emitting it would be a field that can only ever be redundant.
pub fn breakpoint_marker() -> serde_json::Value {
    serde_json::json!({ "mode": "explicit" })
}
