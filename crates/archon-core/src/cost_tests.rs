//! Tests for `cost.rs`.
//!
//! The overrides are a process-global installed once, so nothing here calls
//! `install_pricing_overrides` — a test that did would leak into every other
//! test in the binary and make the order matter.

use super::*;

const EPS: f64 = 1e-9;

#[test]
fn deepseek_keeps_its_own_figures() {
    let cost = estimate_turn_cost_usd("deepseek-v4-pro[1m]", 1_000_000, 1_000_000, 0, 0);

    assert!((cost - 1.305).abs() < EPS, "{cost}");
}

#[test]
fn deepseek_cache_hits_stay_far_below_a_tenth() {
    let cost = estimate_turn_cost_usd("deepseek-v4-pro[1m]", 0, 0, 0, 1_000_000);

    assert!((cost - 0.003625).abs() < EPS, "{cost}");
}

#[test]
fn each_claude_model_is_costed_at_its_own_rate() {
    for (model, input, output) in [
        ("claude-opus-5", 5.0, 25.0),
        ("claude-sonnet-5", 2.0, 10.0),
        ("claude-sonnet-4-6", 3.0, 15.0),
        ("claude-haiku-4-5", 1.0, 5.0),
        ("claude-fable-5", 10.0, 50.0),
    ] {
        let cost = estimate_turn_cost_usd(model, 1_000_000, 1_000_000, 0, 0);
        assert!((cost - (input + output)).abs() < EPS, "{model}: {cost}");
    }
}

/// The bug this rewrite exists for. A read used to be free and a write used to
/// be plain input, so caching could only ever look like a saving.
#[test]
fn cache_reads_and_writes_are_both_priced() {
    let read_only = estimate_turn_cost_usd("claude-sonnet-4-6", 0, 0, 0, 1_000_000);
    let write_only = estimate_turn_cost_usd("claude-sonnet-4-6", 0, 0, 1_000_000, 0);

    assert!(
        (read_only - 0.3).abs() < EPS,
        "a read is not free: {read_only}"
    );
    assert!(
        (write_only - 3.75).abs() < EPS,
        "a write costs 1.25x input: {write_only}"
    );
}

/// A checkpoint written and never read is a loss, and the figures have to say so
/// — this is exactly the shape of the deployment that overspent.
#[test]
fn a_write_that_is_never_read_costs_more_than_not_caching() {
    let cached = estimate_turn_cost_usd("claude-sonnet-4-6", 0, 0, 100_000, 0);
    let uncached = uncached_equivalent_usd("claude-sonnet-4-6", 0, 0, 100_000, 0);

    assert!(
        cached > uncached,
        "cached {cached} must exceed uncached {uncached}"
    );
}

#[test]
fn a_write_read_back_once_pays_for_itself() {
    // One write, then one read of the same prefix.
    let cached = estimate_turn_cost_usd("claude-sonnet-4-6", 0, 0, 100_000, 100_000);
    let uncached = uncached_equivalent_usd("claude-sonnet-4-6", 0, 0, 100_000, 100_000);

    assert!(
        cached < uncached,
        "cached {cached} should beat uncached {uncached} after one read"
    );
}

/// The regional premium is 10% on every token category, so a European Bedrock
/// deployment was being under-reported by a tenth across the board.
#[test]
fn a_regional_bedrock_id_costs_ten_percent_more() {
    let global = estimate_turn_cost_usd("claude-opus-5", 1_000_000, 0, 0, 0);
    let regional = estimate_turn_cost_usd("eu.anthropic.claude-opus-5", 1_000_000, 0, 0, 0);

    assert!((global - 5.0).abs() < EPS, "{global}");
    assert!((regional - 5.5).abs() < EPS, "{regional}");
}

/// The session counter is cumulative and already contains the cached buckets.
/// Charging them again at the full input rate is how a session total ran ahead
/// of the sum of its turns.
#[test]
fn the_session_total_does_not_charge_cached_tokens_twice() {
    // 100k of context, of which 80k was served from cache.
    let session = estimate_session_cost_usd("claude-sonnet-4-6", 100_000, 0, 0, 80_000);
    let expected = 20_000.0 * 3.0 / 1e6 + 80_000.0 * 0.3 / 1e6;

    assert!((session - expected).abs() < EPS, "{session}");
}

#[test]
fn the_session_total_agrees_with_the_turn_it_is_built_from() {
    let turn = estimate_turn_cost_usd("claude-opus-5", 20_000, 1_000, 5_000, 40_000);
    let session = estimate_session_cost_usd("claude-opus-5", 65_000, 1_000, 5_000, 40_000);

    assert!((turn - session).abs() < EPS, "{turn} vs {session}");
}

/// A one-hour checkpoint writes at 2x, not 1.25x. Callers that know say so.
#[test]
fn the_one_hour_tier_writes_at_twice_input() {
    let five_minute = estimate_turn_cost_usd_with_ttl("claude-opus-5", 0, 0, 1_000_000, 0, false);
    let one_hour = estimate_turn_cost_usd_with_ttl("claude-opus-5", 0, 0, 1_000_000, 0, true);

    assert!((five_minute - 6.25).abs() < EPS, "{five_minute}");
    assert!((one_hour - 10.0).abs() < EPS, "{one_hour}");
}

/// An unknown model must still price its cache tiers in the right *shape*, or
/// it inherits the exact bug this replaced.
#[test]
fn an_unrecognised_model_still_prices_the_cache() {
    let read = estimate_turn_cost_usd("some-new-model", 0, 0, 0, 1_000_000);
    let write = estimate_turn_cost_usd("some-new-model", 0, 0, 1_000_000, 0);

    assert!(read > 0.0, "a read is never free");
    assert!(write > estimate_turn_cost_usd("some-new-model", 1_000_000, 0, 0, 0));
}
