//! How a provider wants prompt-cache breakpoints expressed.
//!
//! This replaced a boolean. The boolean asked "is this the official Anthropic
//! endpoint", which conflated two separate questions — *can* this endpoint
//! cache, and *how* does it want to be told — and answered "no caching" for
//! everything else. Gateways, Bedrock and OpenAI all cache; they simply want it
//! said differently, and one of them wants nothing said at all.

use crate::cache_models::CachePlatform;

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

    /// `prompt_cache_breakpoint: {"mode": "explicit"}` on a content block,
    /// paired with a stable `prompt_cache_key`.
    ///
    /// GPT-5.6 and later. `max` is 4 writes per request, `min_tokens` is 1,024
    /// measured through the breakpoint over the whole rendered prefix. There is
    /// no TTL field on the variant because there is no choice to make: the only
    /// accepted value of `prompt_cache_options.ttl` is `30m`, which is also the
    /// default.
    ///
    /// The key is not optional in practice. OpenAI documents that GPT-5.6 needs
    /// `prompt_cache_key` set to get its reliable matching at all, and routes on
    /// a hash of the leading ~256 tokens combined with the key. Archon derives
    /// one from the stable prefix — see [`crate::cache_wire::prompt_cache_key`].
    ///
    /// Emitted on the API path by `OpenAiProvider`. The Codex subscription path
    /// still cannot carry one, and that is a finding rather than an omission —
    /// see the note on `CodexProvider`.
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

    /// The same strategy carrying different limits.
    ///
    /// This is how `[context.prompt_cache_models]` reaches the wire: the
    /// provider names the mechanism, and a configured entry — a model released
    /// after the binary, or a built-in that went stale — replaces the numbers
    /// without touching it. `Automatic` and `None` have no numbers to replace
    /// and pass through unchanged.
    pub fn with_limits(self, params: crate::cache_models::ModelCacheParams) -> Self {
        match self {
            CacheStrategy::AnthropicBreakpoints { .. } => CacheStrategy::AnthropicBreakpoints {
                max: params.max_checkpoints,
                min_tokens: params.min_tokens,
                ttl_1h: params.ttl_1h,
            },
            CacheStrategy::BedrockCachePoint { .. } => CacheStrategy::BedrockCachePoint {
                max: params.max_checkpoints,
                min_tokens: params.min_tokens,
                ttl_1h: params.ttl_1h,
            },
            CacheStrategy::ResponsesBreakpoints { .. } => CacheStrategy::ResponsesBreakpoints {
                max: params.max_checkpoints,
                min_tokens: params.min_tokens,
            },
            CacheStrategy::Automatic | CacheStrategy::None => self,
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

/// The `cache_control` breakpoint limits for a model on a given stack.
///
/// Not a constant, because the minimum is per-model and not inferable from the
/// name: 512 for Opus 5, 1,024 for Sonnet 5 and Sonnet 4.6, 2,048 for Opus 4.7
/// and Haiku 3.5, 4,096 for Opus 4.5, Opus 4.6 and Haiku 4.5. A flat 1,024
/// would silently disable caching on every model in the 2,048 and 4,096 bands,
/// since a checkpoint under the minimum is discarded rather than rejected.
///
/// The platform is a second axis over the same models: Sonnet 4.5 caches from
/// 1,024 tokens on Anthropic's own endpoint and 4,096 on Bedrock, and each
/// operator is the authority on its own service.
pub fn anthropic_for_model(model: &str, platform: CachePlatform) -> CacheStrategy {
    let params = crate::cache_models::ModelCacheTable::default().lookup_on(model, platform);
    CacheStrategy::AnthropicBreakpoints {
        max: params.max_checkpoints,
        min_tokens: params.min_tokens,
        ttl_1h: params.ttl_1h,
    }
}

/// The Anthropic Messages API at its most permissive limits.
///
/// For tests and for callers with no model in hand. Prefer
/// [`anthropic_for_model`] anywhere a model id is available.
pub const ANTHROPIC_API: CacheStrategy = CacheStrategy::AnthropicBreakpoints {
    max: 4,
    min_tokens: 1024,
    ttl_1h: true,
};

/// Key under `LlmRequest::extra` carrying the resolved Bedrock cache directive.
///
/// Converse checkpoints are written at the wire layer, inside the provider —
/// but whether to write them is the operator's decision, made from config the
/// provider never sees (`prompt_cache`, `prompt_cache_mode`,
/// `prompt_cache_ttl`, `[context.prompt_cache_models]`). This key is the
/// channel: `archon-core` resolves all of that into
/// `{"max": n, "min_tokens": n, "ttl_1h": bool}` and attaches it; the provider
/// emits checkpoints only when it is present. Absent means no — a request that
/// never went through the resolution (an SDK caller, a test) gets no
/// checkpoints rather than unconditional ones.
pub const BEDROCK_CACHE_DIRECTIVE_KEY: &str = "prompt_cache_bedrock";

/// Key under `LlmRequest::extra` carrying the stable-system-block boundary.
///
/// Recorded separately from the directive on purpose. The boundary is known by
/// `apply_stable_system_cache`, which runs *before* `apply_conversation_cache`
/// builds the directive — so writing it into the directive would be overwritten
/// moments later. Its own key is order-independent, which matters because the
/// two are called from four different request paths.
pub const STABLE_SYSTEM_BLOCKS_KEY: &str = "prompt_cache_stable_system_blocks";

/// Which Bedrock Converse sections should carry a `cachePoint`.
///
/// Bedrock evaluates the sections in the order `tools` -> `system` ->
/// `messages` and **chains** them: changing an earlier section invalidates the
/// caches of every later one. Checkpoints therefore go after the most stable
/// content first, and the flags exist so a caller can stop short — caching only
/// `tools` and `system` is a legitimate choice when the conversation is not
/// worth a cache write on every turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePointPlacement {
    pub tools: bool,
    pub system: bool,
    pub messages: bool,
    /// Request one-hour retention instead of the default five minutes.
    pub ttl_1h: bool,
    /// How many leading `system` blocks are stable across turns.
    ///
    /// The checkpoint goes **after these**, not after the whole section. Archon
    /// appends per-turn content — recalled memories, the inner voice, reminders
    /// — to the end of the system prompt, and that content changes most turns.
    /// A checkpoint behind it is therefore rewritten every turn and almost never
    /// read back, which is not merely wasted: a cache write bills at 1.25x plain
    /// input (2x at the one-hour TTL), so a checkpoint that never gets read
    /// costs *more* than not caching at all.
    ///
    /// `None` means "no known boundary" and puts the checkpoint at the end, the
    /// old behaviour, which is right when nothing volatile follows.
    pub system_stable_blocks: Option<usize>,
}

impl CachePointPlacement {
    /// Cache every section: three checkpoints, one under Bedrock's limit of
    /// four.
    pub fn all(ttl_1h: bool) -> Self {
        Self {
            tools: true,
            system: true,
            messages: true,
            ttl_1h,
            system_stable_blocks: None,
        }
    }

    /// Place the system checkpoint after the first `blocks` entries.
    pub fn with_stable_system_blocks(mut self, blocks: Option<usize>) -> Self {
        self.system_stable_blocks = blocks;
        self
    }

    /// The checkpoint element itself.
    ///
    /// Converse takes this as its own **array element**, not as an attribute on
    /// a neighbouring block — the difference from every other provider's shape.
    /// `type: "default"` is the only supported value.
    pub fn point(self) -> serde_json::Value {
        if self.ttl_1h {
            serde_json::json!({"cachePoint": {"type": "default", "ttl": "1h"}})
        } else {
            serde_json::json!({"cachePoint": {"type": "default"}})
        }
    }

    /// How many checkpoints this placement emits.
    pub fn count(self) -> usize {
        usize::from(self.tools) + usize::from(self.system) + usize::from(self.messages)
    }
}

/// Parse a configured override.
///
/// `auto` is absent deliberately — it means "ask the provider", which is the
/// caller's decision, not a strategy. Returns `None` for an unrecognised value
/// so the caller can warn rather than silently pick a default that changes
/// spend.
///
/// `platform` is deliberately *not* derived from `value`. The two describe
/// different things and routinely disagree: a LiteLLM proxy in front of Bedrock
/// is declared `anthropic`, because that is the wire format it accepts, while
/// the stack enforcing the minimums is Bedrock. Callers that cannot identify the
/// stack pass [`CachePlatform::Unknown`] and get the strictest reading.
pub fn parse_override(value: &str, model: &str, platform: CachePlatform) -> Option<CacheStrategy> {
    // The limits come from the model and the stack, not the wire format.
    // Declaring an endpoint "anthropic" says how to phrase a breakpoint; it says
    // nothing about how large the prefix must be before one takes effect, and
    // that varies from 512 to 4,096.
    let params = crate::cache_models::ModelCacheTable::default().lookup_on(model, platform);

    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "disabled" => Some(CacheStrategy::None),
        "automatic" => Some(CacheStrategy::Automatic),
        "anthropic" => Some(CacheStrategy::AnthropicBreakpoints {
            max: params.max_checkpoints,
            min_tokens: params.min_tokens,
            ttl_1h: params.ttl_1h,
        }),
        "bedrock" => Some(CacheStrategy::BedrockCachePoint {
            max: params.max_checkpoints,
            min_tokens: params.min_tokens,
            ttl_1h: params.ttl_1h,
        }),
        "responses" => Some(CacheStrategy::ResponsesBreakpoints {
            max: params.max_checkpoints,
            min_tokens: params.min_tokens,
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

    /// The wire format is chosen by config; the limits come from the model and
    /// the stack. Both `anthropic` and `bedrock` on the same model and the same
    /// stack must therefore agree on the minimum — how a breakpoint is phrased
    /// cannot change how many tokens the service needs before honouring one.
    #[test]
    fn the_override_names_the_format_but_the_model_sets_the_limits() {
        for model in ["claude-opus-5", "claude-opus-4-5", "claude-sonnet-4-6"] {
            for platform in [
                CachePlatform::AnthropicApi,
                CachePlatform::Bedrock,
                CachePlatform::Unknown,
            ] {
                let anthropic = parse_override("anthropic", model, platform).unwrap();
                let bedrock = parse_override("bedrock", model, platform).unwrap();
                assert_eq!(
                    anthropic.min_tokens(),
                    bedrock.min_tokens(),
                    "{model} on {platform:?}: the wire format must not change the minimum"
                );
            }
        }
    }

    /// The counterpart: the *stack* may change it, and does. This is the shape
    /// of the deployment that overspent — a proxy declared `anthropic` while
    /// Bedrock sat behind it enforcing a four-times-higher floor.
    #[test]
    fn the_stack_does_change_the_limits_even_at_a_fixed_format() {
        let on_anthropic = parse_override(
            "anthropic",
            "claude-sonnet-4-5",
            CachePlatform::AnthropicApi,
        )
        .unwrap();
        let on_bedrock =
            parse_override("anthropic", "claude-sonnet-4-5", CachePlatform::Bedrock).unwrap();

        assert_eq!(on_anthropic.min_tokens(), 1024);
        assert_eq!(on_bedrock.min_tokens(), 4096);
    }

    /// A gateway is the case archon cannot see through, so it must resolve to
    /// the strictest candidate rather than the friendliest. Sending Anthropic's
    /// 1,024 to a proxy that turns out to front Bedrock puts every checkpoint
    /// under the floor, where it is dropped without an error and billed in full.
    #[test]
    fn an_unidentified_gateway_takes_the_strictest_reading() {
        let gateway =
            parse_override("anthropic", "claude-sonnet-4-5", CachePlatform::Unknown).unwrap();
        assert_eq!(gateway.min_tokens(), 4096, "must assume the higher floor");

        // Same for the TTL, in the other direction: an hour is requested only
        // where every candidate stack accepts one, because asking for it where
        // it is unsupported fails the request outright.
        let ttl_split = parse_override("anthropic", "claude-opus-4-6", CachePlatform::Unknown)
            .expect("opus 4.6");
        assert!(
            !ttl_split.supports_1h_ttl(),
            "4.6's Bedrock TTL is contested, so a gateway that may be fronting \
             Bedrock must not ask for an hour"
        );
    }

    /// The values that cost money when wrong, and the reason a rule cannot
    /// replace the table: these are not ordered by version.
    #[test]
    fn per_model_minimums_are_not_derivable_from_the_version() {
        for (model, expected) in [
            ("claude-opus-5", 512),
            ("claude-opus-4-5", 4096),
            ("claude-sonnet-4-6", 1024),
        ] {
            assert_eq!(
                parse_override("anthropic", model, CachePlatform::AnthropicApi)
                    .unwrap()
                    .min_tokens(),
                expected,
                "{model}"
            );
        }
    }

    /// An unrecognised value must not silently resolve to something that
    /// changes what the deployment spends.
    #[test]
    fn an_unknown_override_is_rejected_rather_than_defaulted() {
        assert_eq!(
            parse_override("enabled", "claude-opus-5", CachePlatform::AnthropicApi),
            None
        );
        assert_eq!(
            parse_override("auto", "claude-opus-5", CachePlatform::AnthropicApi),
            None
        );
    }

    #[test]
    fn overrides_round_trip_through_their_names() {
        for name in ["off", "automatic", "anthropic", "bedrock", "responses"] {
            let parsed =
                parse_override(name, "claude-sonnet-4-6", CachePlatform::AnthropicApi).expect(name);
            assert_eq!(parsed.as_str(), name, "{name}");
        }
    }
}
