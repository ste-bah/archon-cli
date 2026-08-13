//! The built-in per-model cache parameters.
//!
//! Split from `cache_models.rs` for the 500-line gate. Data only — the lookup
//! rules that consume it live next door.

use crate::cache_models::ModelCacheParams;

/// Built-in defaults, matched by substring against a model id.
///
/// Substrings rather than exact ids because the same model arrives in several
/// shapes: bare (`anthropic.claude-sonnet-4-5-...`), region-prefixed
/// (`eu.anthropic...`, `us.anthropic...`), and as inference-profile ARNs. None
/// of the latter start with `anthropic.`.
///
/// Order is **not** load-bearing: [`ModelCacheTable::lookup`] takes the longest
/// matching marker, so a specific entry beats a shorter one it contains no
/// matter where either sits. The grouping below is for reading, nothing more.
///
/// # Where these numbers come from, and why they cannot be derived
///
/// Minimums are **not monotonic in the model name**: Opus 4.5 and 4.6 need
/// 4,096 while Opus 5 needs 512, and Sonnet 4.6 needs 1,024 while its
/// same-generation siblings need 4,096. Any rule inferring a minimum from a
/// version number gets several of these wrong, so each is listed individually.
///
/// AWS's prompt-caching page is also **not** the complete Bedrock list — it
/// carries only models absent from "models at a glance", so Opus 5 and Sonnet 5
/// appear solely in their per-model cards. Seeding this table from that page
/// alone silently omits them.
pub const BUILT_IN_MODELS: &[(&str, ModelCacheParams)] = &[
    // --- Claude 5 generation -------------------------------------------------
    // Opus 5 at 512 is agreed by the AWS model card, Anthropic's docs and
    // LiteLLM's dataset.
    (
        "claude-opus-5",
        ModelCacheParams {
            min_tokens: 512,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    // Anthropic documents 1,024 and LiteLLM agrees; an AWS model card says
    // 4,096. Not a contradiction to be resolved — they describe different
    // endpoints, and each operator governs its own.
    (
        "claude-sonnet-5",
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: Some(4096),
            bedrock_ttl_1h: None,
        },
    ),
    // 512-token minimum, same band as Opus 5.
    (
        "claude-fable-5",
        ModelCacheParams {
            min_tokens: 512,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    (
        "claude-mythos-preview",
        ModelCacheParams {
            min_tokens: 2048,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    (
        "claude-mythos-5",
        ModelCacheParams {
            min_tokens: 512,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    // --- 4.5 generation: 4,096 minimum ---------------------------------------
    (
        "claude-opus-4-5",
        ModelCacheParams {
            min_tokens: 4096,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    // Anthropic says 1,024; AWS's caching table says 4,096 for its own endpoint.
    (
        "claude-sonnet-4-5",
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: Some(4096),
            bedrock_ttl_1h: None,
        },
    ),
    (
        "claude-haiku-4-5",
        ModelCacheParams {
            min_tokens: 4096,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    // One-hour TTL on the Anthropic API; on Bedrock the sources disagree and
    // this takes the losing side deliberately.
    //
    // AWS's prompt-caching userguide names only Opus 4.5, Haiku 4.5 and Sonnet
    // 4.5 as accepting `"ttl": "1h"`. The `aws-samples/amazon-bedrock-samples`
    // matrix additionally lists Opus 4.6 and Sonnet 4.6 as supporting it, and is
    // the more recently maintained of the two — the userguide sentence reads
    // like it was written for the 4.5 launch and never revised.
    //
    // Unresolved, so the asymmetry decides. Requesting an hour where it is
    // unsupported fails the request outright, on every turn; requesting five
    // minutes where an hour was available costs a shorter cache and nothing
    // else. If the samples matrix is right, this leaves a saving on the table —
    // recoverable in one line via `bedrock_ttl_1h` in
    // `[context.prompt_cache_models]`, without a release. Worth revisiting once
    // AWS states it in one place.
    (
        "claude-opus-4-6",
        ModelCacheParams {
            min_tokens: 4096,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: Some(false),
        },
    ),
    // The exception in its generation: back down to 1,024. Grouping it with its
    // siblings by version number would be wrong. Same contested Bedrock TTL as
    // Opus 4.6 above, resolved the same conservative way.
    (
        "claude-sonnet-4-6",
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: Some(false),
        },
    ),
    // --- 2,048 minimum -------------------------------------------------------
    // Anthropic's own table; neither is on AWS's caching page.
    (
        "claude-opus-4-7",
        ModelCacheParams {
            min_tokens: 2048,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    (
        "claude-3-5-haiku",
        ModelCacheParams {
            min_tokens: 2048,
            max_checkpoints: 4,
            ttl_1h: false,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    // --- 1,024 minimum -------------------------------------------------------
    (
        "claude-opus-4-8",
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    // Opus 4.1 and Sonnet 4 are retired on the first-party API and absent from
    // AWS's caching table, so neither vendor documents their TTL support. The
    // minimum is documented; the TTL is not, and requesting an hour where it is
    // unsupported fails the request outright — so it is left off rather than
    // guessed.
    (
        "claude-opus-4-1",
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: false,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    (
        "claude-sonnet-4",
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: false,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    (
        "claude-opus-4",
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: false,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    // Google withholds the extended TTL for this model and the 3.5 Sonnets on
    // Vertex. No `vertex_*` override is needed to express that, because neither
    // Anthropic nor AWS documents a one-hour TTL for them either — the flag is
    // already off on every stack.
    (
        "claude-3-7-sonnet",
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: false,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    (
        "claude-3-5-sonnet",
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: false,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
    // --- OpenAI: Responses-API breakpoints -----------------------------------
    // The TTL is thirty minutes, which `ttl_1h` cannot express and does not need
    // to: it is not selectable, so there is no request to get wrong.
    (
        "gpt-5.6",
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: false,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    ),
];
