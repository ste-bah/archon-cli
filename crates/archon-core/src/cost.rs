//! Turn and session cost estimates from provider-reported usage.
//!
//! What this replaced priced every non-DeepSeek model at a flat Sonnet estimate,
//! charged cache reads at **zero**, and folded cache writes into plain input at
//! **1.0x**. Each of those is wrong in the same direction: it makes caching look
//! free. A deployment writing checkpoints it never read back showed a falling
//! per-turn cost while the invoice climbed, which is precisely the shape of the
//! overspend that opened #178.
//!
//! Cache reads and writes are now priced from their published multipliers of
//! base input — see [`crate::cost_table`] — so a checkpoint that does not pay
//! for itself shows up as the loss it is.

use std::collections::BTreeMap;
use std::sync::OnceLock;

pub use crate::cost_table::Pricing;
use crate::cost_table::{UNKNOWN_MODEL_ESTIMATE, lookup, platform_multiplier};

const TOKENS_PER_MILLION: f64 = 1_000_000.0;

/// Operator-supplied prices, keyed by model-id substring, matched the same way
/// as the built-in table.
static OVERRIDES: OnceLock<BTreeMap<String, Pricing>> = OnceLock::new();

/// Install `[context.model_pricing]` for the life of the process.
///
/// A global rather than a parameter because the cost functions are called from
/// the TUI event loop, the status line and two slash commands, none of which
/// hold configuration — and threading it to all of them to change a *displayed
/// estimate* would be a large change for a small one. Installed once at startup;
/// later calls are ignored, so a subagent cannot silently reprice a session.
///
/// The knob exists for the same reason the cache-parameter one does: a model
/// released after the binary, or a vendor revising a figure, should not require
/// a release to cost correctly.
pub fn install_pricing_overrides(overrides: BTreeMap<String, Pricing>) {
    if overrides.is_empty() {
        return;
    }
    let _ = OVERRIDES.set(overrides);
}

/// Prices in force for a model, including any regional platform premium.
pub fn pricing_for_model(model: &str) -> Pricing {
    let normalized = model.trim().to_ascii_lowercase();
    let configured = OVERRIDES.get().and_then(|overrides| {
        overrides
            .iter()
            .filter(|(marker, _)| normalized.contains(marker.trim().to_ascii_lowercase().as_str()))
            .max_by_key(|(marker, _)| marker.len())
            .map(|(_, pricing)| *pricing)
    });

    let base = configured
        .or_else(|| lookup(model))
        .unwrap_or(UNKNOWN_MODEL_ESTIMATE);
    base.scaled(platform_multiplier(model))
}

/// Estimate the cost of one turn from the usage the provider reported.
///
/// `input_tokens` is the non-cache input for this turn. Bedrock's `inputTokens`
/// already excludes cached tokens — verified live, `inputTokens: 3` on a
/// 4,424-token request — and the Anthropic Messages API reports the same split,
/// so the three buckets do not overlap and are simply summed.
///
/// The five-minute write tier is assumed. `prompt_cache_ttl = "1h"` writes at
/// 2x rather than 1.25x, which this understates; the counters the providers
/// return do not distinguish the two, so the alternative would be to guess in
/// the more expensive direction on every deployment that never asked for an
/// hour. See [`estimate_turn_cost_usd_with_ttl`] where the caller does know.
pub fn estimate_turn_cost_usd(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
) -> f64 {
    estimate_turn_cost_usd_with_ttl(
        model,
        input_tokens,
        output_tokens,
        cache_creation_tokens,
        cache_read_tokens,
        false,
    )
}

/// As [`estimate_turn_cost_usd`], for a caller that knows the retention tier.
pub fn estimate_turn_cost_usd_with_ttl(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    ttl_1h: bool,
) -> f64 {
    let pricing = pricing_for_model(model);
    per_mtok(input_tokens, pricing.input_per_mtok)
        + per_mtok(output_tokens, pricing.output_per_mtok)
        + per_mtok(cache_creation_tokens, pricing.cache_write_per_mtok(ttl_1h))
        + per_mtok(cache_read_tokens, pricing.cache_read_per_mtok())
}

/// Estimate cumulative session cost from the running counters.
///
/// `context_input_tokens` is the cumulative input total, which *includes* the
/// cached buckets — so they are subtracted before the remainder is priced as
/// plain input, or the same tokens would be charged twice. The previous version
/// did charge them twice for every provider except DeepSeek, at the full input
/// rate, while separately reporting the cache as free.
pub fn estimate_session_cost_usd(
    model: &str,
    context_input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
) -> f64 {
    let cached = cache_creation_tokens.saturating_add(cache_read_tokens);
    let uncached_input = context_input_tokens.saturating_sub(cached);

    estimate_turn_cost_usd(
        model,
        uncached_input,
        output_tokens,
        cache_creation_tokens,
        cache_read_tokens,
    )
}

/// What a turn would have cost with no caching at all.
///
/// The comparison that makes a checkpoint's worth legible: every cached token,
/// read or written, would otherwise have been plain input. Against
/// [`estimate_turn_cost_usd`] this shows whether the cache paid for itself —
/// and a write that is never read back makes this figure the *smaller* of the
/// two.
pub fn uncached_equivalent_usd(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
) -> f64 {
    let pricing = pricing_for_model(model);
    let all_input = input_tokens
        .saturating_add(cache_creation_tokens)
        .saturating_add(cache_read_tokens);

    per_mtok(all_input, pricing.input_per_mtok) + per_mtok(output_tokens, pricing.output_per_mtok)
}

fn per_mtok(tokens: u64, price_per_mtok: f64) -> f64 {
    tokens as f64 * price_per_mtok / TOKENS_PER_MILLION
}

#[cfg(test)]
#[path = "cost_tests.rs"]
mod tests;
