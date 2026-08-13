//! `[context]` — the section that decides what reaches the model and what it
//! costs.
//!
//! Split from `sections.rs` for the 500-line gate; it grew the prompt-cache
//! and pricing knobs in #178 and was the largest section by some margin.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfig {
    pub compact_threshold: f32,
    pub preflight_safety_margin: f32,
    pub max_tokens: Option<u32>,
    pub context_window_override: Option<u64>,
    pub output_reserve_tokens: u64,
    pub preserve_recent_turns: u32,
    pub manual_compact_force_strategy: String,
    pub rate_limit_pressure_tokens: Option<u64>,
    pub rate_limit_pressure_body_bytes: Option<u64>,
    pub large_request_retry_body_bytes: Option<u64>,
    /// Maximum serialized JSON bytes for any individual provider-facing tool result field.
    pub max_tool_result_bytes: usize,
    /// Optional explicit model used by background segment summarization.
    pub compaction_model: Option<String>,
    /// Whether to use prompt caching (cache_control breakpoints on static blocks).
    pub prompt_cache: bool,
    pub prompt_cache_mode: String,
    pub prompt_cache_ttl: String,
    pub prompt_cache_conversation: bool,
    /// Move the per-turn system blocks behind the message history.
    ///
    /// Archon appends recalled memories, the inner voice and the turn's
    /// reminders to the end of the system prompt, which puts volatile content
    /// *in front of* the entire conversation. Every provider caches by common
    /// prefix, so one changed memory invalidates the whole history behind it —
    /// and on the providers that cache implicitly (GPT-5.5 and earlier,
    /// DeepSeek) there is no breakpoint to place and this ordering is the only
    /// lever there is.
    ///
    /// Moving those blocks onto the last user message, where archon's own
    /// `<system-reminder>` content already goes, leaves the stable system
    /// prompt, the tools and the whole history as one uninterrupted prefix.
    ///
    /// It does change where the model sees that text — later, and closer to the
    /// user's message — which is why it is a switch rather than an assumption.
    pub prompt_cache_reorder: bool,
    /// Which cache wire format the endpoint accepts, when it cannot be inferred.
    ///
    /// `"auto"` asks the provider, which recognises the official Anthropic
    /// Messages endpoint and nothing else — a gateway URL tells archon nothing
    /// about what is behind it, so caching stays off and every request is
    /// billed in full. Set this explicitly to declare a known-good endpoint.
    ///
    /// `auto` | `anthropic` | `bedrock` | `responses` | `automatic` | `off`
    pub prompt_cache_strategy: String,
    /// Per-model cache parameters, keyed by a model-id substring; see the
    /// `[context.prompt_cache_models]` template in `config.toml`.
    ///
    /// No provider exposes these through an API, so Archon ships a built-in
    /// table; this overrides and extends it, which is what stops a model
    /// released after the binary from being stuck with a guessed minimum until
    /// the next release. Applied in `request_cache::resolve_strategy`, the
    /// choke point every request passes through.
    #[serde(default)]
    pub prompt_cache_models:
        std::collections::BTreeMap<String, archon_llm::cache_models::ModelCacheParams>,
    /// Per-model token prices, keyed by a model-id substring, overriding and
    /// extending the built-in table in `cost_table.rs`.
    ///
    /// Same reasoning as `prompt_cache_models`: a model released after the
    /// binary, or a vendor revising a figure, should not need a release to be
    /// costed correctly. `input_per_mtok` and `output_per_mtok` are required;
    /// the three cache multipliers default to Claude's published ratios
    /// (0.1x read, 1.25x write, 2x for the one-hour tier).
    #[serde(default)]
    pub model_pricing: std::collections::BTreeMap<String, crate::cost_table::Pricing>,
    /// Maximum characters for hierarchical ARCHON.md content.
    #[serde(alias = "claudemd_max_tokens")]
    pub archonmd_max_tokens: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            compact_threshold: 0.80,
            preflight_safety_margin: 0.05,
            max_tokens: None,
            context_window_override: None,
            output_reserve_tokens: 8192,
            preserve_recent_turns: 3,
            manual_compact_force_strategy: "micro".into(),
            rate_limit_pressure_tokens: Some(120_000),
            rate_limit_pressure_body_bytes: Some(320_000),
            large_request_retry_body_bytes: Some(320_000),
            max_tool_result_bytes: crate::agent::tool_result_context::DEFAULT_MAX_TOOL_RESULT_BYTES,
            compaction_model: None,
            prompt_cache: true,
            prompt_cache_mode: "explicit".into(),
            prompt_cache_ttl: "5m".into(),
            prompt_cache_conversation: true,
            prompt_cache_reorder: true,
            prompt_cache_strategy: "auto".into(),
            prompt_cache_models: std::collections::BTreeMap::new(),
            model_pricing: std::collections::BTreeMap::new(),
            archonmd_max_tokens: 8192,
        }
    }
}
