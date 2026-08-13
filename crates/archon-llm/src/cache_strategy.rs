//! How a provider wants prompt-cache breakpoints expressed.
//!
//! This replaced a boolean. The boolean asked "is this the official Anthropic
//! endpoint", which conflated two separate questions — *can* this endpoint
//! cache, and *how* does it want to be told — and answered "no caching" for
//! everything else. Gateways, Bedrock and OpenAI all cache; they simply want it
//! said differently, and one of them wants nothing said at all.

/// The wire mechanism a provider uses for prompt caching.
///
/// Carrying the limits on the variant rather than looking them up at the call
/// site keeps the rules next to the mechanism they belong to. Both matter:
/// exceeding `max` is a 400, and falling under `min_tokens` is worse — the
/// checkpoint is *silently ignored*, so the request costs full price and looks
/// like it was cached correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStrategy {
    /// The provider caches automatically on a stable prefix; there is nothing
    /// to emit.
    ///
    /// OpenAI (>= 1,024 tokens), GPT-5.5 and earlier. Not a no-op branch: it
    /// means prefix stability is the *only* lever, so anything volatile near
    /// the front of the prompt is the whole cost, and no annotation can rescue
    /// it.
    Automatic,

    /// `cache_control` on a content block.
    ///
    /// The Anthropic Messages API, Anthropic-compatible gateways, and Bedrock's
    /// InvokeModel path for Claude.
    AnthropicBreakpoints {
        max: usize,
        min_tokens: usize,
        ttl_1h: bool,
    },

    /// `{"cachePoint": {"type": "default"}}` as its own **array element**, not
    /// an attribute on a neighbouring block.
    ///
    /// Bedrock's Converse API. Its sections are chained and evaluated in the
    /// order `tools` -> `system` -> `messages`, so changing an earlier section
    /// invalidates every later one, and `min_tokens` is measured against all
    /// three *combined* rather than per section.
    BedrockCachePoint {
        max: usize,
        min_tokens: usize,
        ttl_1h: bool,
    },

    /// `prompt_cache_breakpoint` on a content block, paired with a stable
    /// `prompt_cache_key`.
    ///
    /// GPT-5.6 and later on the Responses API. The key is not optional in
    /// practice: without it the service falls back to weaker matching.
    ResponsesBreakpoints { max: usize, min_tokens: usize },

    /// Nothing is known about this endpoint, so strip any directives rather
    /// than risk a 400 on every request.
    ///
    /// The conservative default, and the behaviour every non-Anthropic provider
    /// had before this type existed.
    None,
}

impl CacheStrategy {
    /// Whether anything at all should be emitted.
    ///
    /// `Automatic` answers **false** here, which is the point of the variant:
    /// the provider caches, but adding markers would be wrong rather than
    /// merely useless.
    pub fn emits_breakpoints(self) -> bool {
        !matches!(self, CacheStrategy::None | CacheStrategy::Automatic)
    }

    /// Whether the provider caches at all, however it is expressed. Used for
    /// reporting, so an `Automatic` provider is not shown as uncached.
    pub fn caches(self) -> bool {
        !matches!(self, CacheStrategy::None)
    }

    /// Maximum breakpoints per request; `0` where none are emitted.
    pub fn max_breakpoints(self) -> usize {
        match self {
            CacheStrategy::AnthropicBreakpoints { max, .. }
            | CacheStrategy::BedrockCachePoint { max, .. }
            | CacheStrategy::ResponsesBreakpoints { max, .. } => max,
            CacheStrategy::Automatic | CacheStrategy::None => 0,
        }
    }

    /// Smallest cacheable prefix. A breakpoint below this is discarded by the
    /// provider without complaint.
    pub fn min_tokens(self) -> usize {
        match self {
            CacheStrategy::AnthropicBreakpoints { min_tokens, .. }
            | CacheStrategy::BedrockCachePoint { min_tokens, .. }
            | CacheStrategy::ResponsesBreakpoints { min_tokens, .. } => min_tokens,
            CacheStrategy::Automatic | CacheStrategy::None => 0,
        }
    }

    /// Whether a one-hour TTL can be requested, rather than the default five
    /// minutes.
    pub fn supports_1h_ttl(self) -> bool {
        match self {
            CacheStrategy::AnthropicBreakpoints { ttl_1h, .. }
            | CacheStrategy::BedrockCachePoint { ttl_1h, .. } => ttl_1h,
            // The Responses API fixes its own retention; there is no knob.
            CacheStrategy::Automatic
            | CacheStrategy::ResponsesBreakpoints { .. }
            | CacheStrategy::None => false,
        }
    }

    /// Name used by config and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            CacheStrategy::Automatic => "automatic",
            CacheStrategy::AnthropicBreakpoints { .. } => "anthropic",
            CacheStrategy::BedrockCachePoint { .. } => "bedrock",
            CacheStrategy::ResponsesBreakpoints { .. } => "responses",
            CacheStrategy::None => "off",
        }
    }
}

/// The Anthropic Messages API's own limits.
pub const ANTHROPIC_API: CacheStrategy = CacheStrategy::AnthropicBreakpoints {
    max: 4,
    min_tokens: 1024,
    ttl_1h: true,
};

/// Parse a configured override.
///
/// `auto` is absent deliberately — it means "ask the provider", which is the
/// caller's decision, not a strategy. Returns `None` for an unrecognised value
/// so the caller can warn rather than silently pick a default that changes
/// spend.
pub fn parse_override(value: &str) -> Option<CacheStrategy> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "disabled" => Some(CacheStrategy::None),
        "automatic" => Some(CacheStrategy::Automatic),
        "anthropic" => Some(ANTHROPIC_API),
        "bedrock" => Some(CacheStrategy::BedrockCachePoint {
            max: 4,
            // The conservative choice across the Bedrock Claude range: the
            // 4.5-generation models require 4,096 and silently drop anything
            // smaller, so assuming 1,024 would produce checkpoints that never
            // fire on the newest models.
            min_tokens: 4096,
            ttl_1h: true,
        }),
        "responses" => Some(CacheStrategy::ResponsesBreakpoints {
            max: 4,
            min_tokens: 1024,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Automatic` caches but must not emit anything. Conflating the two is how
    /// a provider that caches perfectly well ends up with markers it rejects.
    #[test]
    fn automatic_caches_without_emitting() {
        assert!(CacheStrategy::Automatic.caches());
        assert!(!CacheStrategy::Automatic.emits_breakpoints());
        assert_eq!(CacheStrategy::Automatic.max_breakpoints(), 0);
    }

    #[test]
    fn none_neither_caches_nor_emits() {
        assert!(!CacheStrategy::None.caches());
        assert!(!CacheStrategy::None.emits_breakpoints());
    }

    #[test]
    fn breakpoint_strategies_emit() {
        for strategy in [
            ANTHROPIC_API,
            CacheStrategy::BedrockCachePoint {
                max: 4,
                min_tokens: 4096,
                ttl_1h: true,
            },
            CacheStrategy::ResponsesBreakpoints {
                max: 4,
                min_tokens: 1024,
            },
        ] {
            assert!(strategy.emits_breakpoints(), "{strategy:?}");
            assert_eq!(strategy.max_breakpoints(), 4, "{strategy:?}");
        }
    }

    /// The Responses API has no TTL knob; claiming otherwise would put an
    /// unsupported field on the wire.
    #[test]
    fn only_the_block_strategies_offer_a_one_hour_ttl() {
        assert!(ANTHROPIC_API.supports_1h_ttl());
        assert!(
            !CacheStrategy::ResponsesBreakpoints {
                max: 4,
                min_tokens: 1024
            }
            .supports_1h_ttl()
        );
    }

    /// Bedrock's newest Claude models need 4,096 tokens and silently ignore a
    /// smaller checkpoint, so the override must not assume the 1,024 that the
    /// Anthropic API allows.
    #[test]
    fn the_bedrock_override_assumes_the_higher_minimum() {
        assert_eq!(parse_override("bedrock").unwrap().min_tokens(), 4096);
        assert_eq!(parse_override("anthropic").unwrap().min_tokens(), 1024);
    }

    /// An unrecognised value must not silently resolve to something that
    /// changes what the deployment spends.
    #[test]
    fn an_unknown_override_is_rejected_rather_than_defaulted() {
        assert_eq!(parse_override("enabled"), None);
        assert_eq!(parse_override("auto"), None);
    }

    #[test]
    fn overrides_round_trip_through_their_names() {
        for name in ["off", "automatic", "anthropic", "bedrock", "responses"] {
            let parsed = parse_override(name).expect(name);
            assert_eq!(parsed.as_str(), name, "{name}");
        }
    }
}
