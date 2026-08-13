//! Per-model prompt-cache parameters, as data rather than code.
//!
//! # Why this is a table, and why it is configurable
//!
//! No provider exposes these numbers through an API. Bedrock's
//! `ListFoundationModels` and `GetFoundationModel` return modalities, streaming
//! support, customisation options and lifecycle status — nothing about cache
//! minimums, checkpoint limits or TTL. The values live only in documentation.
//!
//! So a table is unavoidable. What is avoidable is having it compiled in: a
//! model released after a build would otherwise be stuck with whatever the
//! fallback guessed until someone shipped a new binary. Entries here are
//! defaults; `[cache.models]` in config.toml overrides and extends them, so a
//! new model is a config edit rather than a release.
//!
//! # Which way it fails matters more than the values
//!
//! A checkpoint below a model's minimum is **silently ignored**. The request
//! succeeds, is billed in full, and is indistinguishable from a cache hit. A
//! checkpoint above the minimum merely starts caching a little later. Every
//! default here is therefore chosen to fail toward "caches later" rather than
//! "never caches, invisibly".

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Which inference stack is serving the model.
///
/// The parameters are not a property of the model alone. Anthropic states its
/// minimums "apply on every platform", but AWS runs its own stack and documents
/// different figures for several models — and for traffic to Bedrock, the
/// operator of that endpoint is the authority on it, whatever the model vendor
/// asserts. Treating them as one number means being wrong on one of them.
///
/// Archon reaches models by seven access paths, which collapse to four stacks:
///
/// | Access path | Stack |
/// |---|---|
/// | Anthropic API key | `AnthropicApi` |
/// | Anthropic subscription (OAuth, Claude Code identity) | `AnthropicApi` |
/// | Claude on Amazon Bedrock | `Bedrock` |
/// | Claude on Google Vertex | `Vertex` |
/// | OpenAI API key | `OpenAiApi` |
/// | OpenAI via Codex subscription | `OpenAiApi` |
/// | GPT-5.6 on Bedrock | `Bedrock` |
///
/// Paths sharing a stack share its thresholds; only the credential differs. The
/// cache itself is isolated per workspace or organisation, so an API-key session
/// and a subscription session do not share *entries* — but the limits governing
/// when a checkpoint takes effect are the same, which is all this type decides.
///
/// A gateway is [`CachePlatform::Unknown`], and defaults there, because the
/// stack behind it cannot be read off the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CachePlatform {
    /// The first-party Anthropic Messages API — by API key or by subscription
    /// OAuth — and gateways positively known to forward to it unchanged.
    AnthropicApi,
    /// Amazon Bedrock, whichever API surface.
    Bedrock,
    /// Google Vertex AI.
    Vertex,
    /// OpenAI's own API, and the Codex subscription that shares it.
    OpenAiApi,
    /// A gateway whose backing stack archon cannot identify.
    ///
    /// The default, and not a placeholder — it is the case that cost £4.5k in a
    /// day. A LiteLLM proxy in front of Bedrock is configured as `anthropic`,
    /// because that names the *wire format* it accepts, and it translates to
    /// Converse `cachePoint` itself. So the declared format says nothing about
    /// the stack: an endpoint calling itself Anthropic may be Bedrock, where
    /// Sonnet 4.5 needs 4,096 tokens rather than 1,024.
    ///
    /// Resolves to the strictest figure across every stack that could be behind
    /// it — the highest minimum, and an extended TTL only where every candidate
    /// allows one. Guessing high starts caching slightly late; guessing low
    /// produces a checkpoint that is discarded in silence and billed in full.
    #[default]
    Unknown,
}

/// Cache parameters for one model, as carried in config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCacheParams {
    /// Smallest cacheable prefix, in tokens. Below this a checkpoint is
    /// discarded by the provider without an error.
    pub min_tokens: usize,
    /// Maximum checkpoints per request.
    pub max_checkpoints: usize,
    /// Whether a one-hour TTL may be requested.
    ///
    /// Asking for one where it is unsupported fails the request outright, so
    /// this defaults to false and is set only where documented.
    #[serde(default)]
    pub ttl_1h: bool,
    /// Bedrock's minimum, where AWS documents one that differs.
    ///
    /// `None` means the platforms agree, which is the usual case. Set only for
    /// the handful where they do not — AWS says Sonnet 4.5 needs 4,096 where
    /// Anthropic says 1,024, for instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bedrock_min_tokens: Option<usize>,
    /// Bedrock's one-hour TTL support, where it differs.
    ///
    /// Opus 4.6 and Sonnet 4.6 accept a one-hour TTL on the Anthropic API and
    /// explicitly do not on Bedrock, so a single flag is wrong for one of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bedrock_ttl_1h: Option<bool>,
}

pub use crate::cache_models_table::BUILT_IN_MODELS;

/// Assumed for a model that matches no entry.
///
/// The higher minimum deliberately, and no one-hour TTL. Guessing low produces
/// checkpoints that vanish in silence; guessing high produces caching that
/// begins slightly later. Only one of those is invisible on the bill.
pub const CONSERVATIVE_DEFAULT: ModelCacheParams = ModelCacheParams {
    min_tokens: 4096,
    max_checkpoints: 4,
    ttl_1h: false,
    bedrock_min_tokens: None,
    bedrock_ttl_1h: None,
};

/// The built-in table plus any configured overrides.
///
/// Configured entries are consulted first, so an operator can correct a stale
/// built-in or add a model that did not exist when the binary was built,
/// without waiting for a release.
#[derive(Debug, Clone, Default)]
pub struct ModelCacheTable {
    overrides: BTreeMap<String, ModelCacheParams>,
}

impl ModelCacheParams {
    /// Resolve to the figures that apply on `platform`.
    ///
    /// Idempotent: the per-platform overrides are consumed here, so resolving a
    /// second time cannot yield a third answer.
    pub fn on(self, platform: CachePlatform) -> Self {
        let resolved = match platform {
            CachePlatform::AnthropicApi => self,
            CachePlatform::Bedrock => Self {
                min_tokens: self.bedrock_min_tokens.unwrap_or(self.min_tokens),
                ttl_1h: self.bedrock_ttl_1h.unwrap_or(self.ttl_1h),
                ..self
            },
            // Vertex serves Anthropic's own figures. Google does withhold the
            // extended TTL for Claude 3.7 Sonnet, 3.5 Sonnet v2, 3.5 Sonnet and
            // 3 Opus — but every one of those already carries `ttl_1h: false`
            // above, because no vendor documents an hour for them anywhere. A
            // `vertex_ttl_1h` override would therefore be a config field that
            // nothing ever sets, so there isn't one. Add it with the first model
            // that genuinely diverges.
            CachePlatform::Vertex => self,
            // OpenAI's own API and the Codex subscription are one stack, and
            // GPT-5.6's breakpoint limits are the same again on Bedrock —
            // 1,024 tokens, four breakpoints, a fixed thirty-minute TTL. Only
            // the billing differs, and billing is not a threshold.
            CachePlatform::OpenAiApi => self,
            // Strictest across every stack that could be behind the gateway.
            CachePlatform::Unknown => Self {
                min_tokens: self.min_tokens.max(self.bedrock_min_tokens.unwrap_or(0)),
                ttl_1h: self.ttl_1h && self.bedrock_ttl_1h.unwrap_or(self.ttl_1h),
                ..self
            },
        };
        Self {
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
            ..resolved
        }
    }
}

impl ModelCacheTable {
    /// Build from `[cache.models]`, keyed by model-id substring.
    pub fn from_config(overrides: BTreeMap<String, ModelCacheParams>) -> Self {
        Self { overrides }
    }

    /// Parameters from the configured overrides alone, ignoring the built-ins.
    ///
    /// This is the question `resolve_strategy` asks: "did the operator say
    /// something about this model?" A full lookup cannot answer it, because the
    /// built-ins would match too and a no-op rewrite is indistinguishable from a
    /// real one.
    ///
    /// Longest-key-first, so a specific entry beats a general one regardless of
    /// the order a TOML table happens to iterate in — map ordering is not
    /// something a config author should have to think about.
    pub fn lookup_configured(&self, model_id: &str) -> Option<ModelCacheParams> {
        let id = model_id.to_ascii_lowercase();
        self.overrides
            .iter()
            .filter(|(marker, _)| id.contains(&marker.to_ascii_lowercase()))
            .max_by_key(|(marker, _)| marker.len())
            .map(|(_, params)| *params)
    }

    /// Parameters for a model id, or `None` if nothing matches.
    pub fn lookup(&self, model_id: &str) -> Option<ModelCacheParams> {
        let id = model_id.to_ascii_lowercase();

        if let Some(params) = self.lookup_configured(model_id) {
            return Some(params);
        }

        // Longest match wins, rather than first-in-source-order.
        //
        // `claude-opus-4` is a substring of `claude-opus-4-1`, `-4-5`, `-4-6`,
        // `-4-7` and `-4-8`, so a first-match scan is correct only while the
        // specific entries happen to sit above the general one. Reordering the
        // list would then collapse Opus 4.5 from 4,096 to 1,024 — a silent
        // cache miss, with nothing failing. Choosing by marker length removes
        // that dependence on source order entirely, and means a future
        // `claude-opus-4-9` cannot be swallowed by `claude-opus-4` either.
        BUILT_IN_MODELS
            .iter()
            .filter(|(marker, _)| id.contains(marker))
            .max_by_key(|(marker, _)| marker.len())
            .map(|(_, params)| *params)
    }

    /// Parameters for a model id, falling back to the conservative default.
    ///
    /// Resolves against the Anthropic API's figures. Use
    /// [`ModelCacheTable::lookup_on`] where the platform is known — the two
    /// differ for several models, and Bedrock is the authority on its own
    /// endpoint.
    pub fn lookup_or_conservative(&self, model_id: &str) -> ModelCacheParams {
        self.lookup_on(model_id, CachePlatform::AnthropicApi)
    }

    /// Parameters for a model id as they apply on `platform`.
    pub fn lookup_on(&self, model_id: &str, platform: CachePlatform) -> ModelCacheParams {
        self.lookup(model_id)
            .unwrap_or(CONSERVATIVE_DEFAULT)
            .on(platform)
    }
}

#[cfg(test)]
#[path = "cache_models_tests.rs"]
mod tests;
