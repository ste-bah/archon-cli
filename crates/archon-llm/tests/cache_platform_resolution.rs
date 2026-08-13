//! How per-model cache parameters resolve against the stack serving them.
//!
//! Split from the table's own unit tests because it asserts a different thing.
//! Those pin the numbers; these pin the second axis over them — the same model
//! caches from 1,024 tokens on Anthropic's endpoint and 4,096 on Bedrock, and
//! each operator is the authority on its own service.
//!
//! Which way this fails is the whole point. A checkpoint *below* a model's
//! minimum is discarded without an error: the request succeeds, is billed in
//! full, and is indistinguishable from a cache hit. A checkpoint above it merely
//! starts caching later. So every assertion here that concerns an unknown or
//! ambiguous endpoint expects the stricter figure.

use archon_llm::cache_models::{CachePlatform, ModelCacheTable};

fn table() -> ModelCacheTable {
    ModelCacheTable::default()
}

/// The vendors disagree about Sonnet 4.5 and Sonnet 5, and both are right about
/// their own endpoint. Sending Anthropic's 1,024 to Bedrock puts the checkpoint
/// under AWS's floor, where it is dropped in silence and billed in full — which
/// is the exact shape of the overspend this work exists to fix.
#[test]
fn the_sonnet_minimum_follows_the_endpoint_being_called() {
    for id in [
        "anthropic.claude-sonnet-4-5-20250929-v1:0",
        "anthropic.claude-sonnet-5",
    ] {
        assert_eq!(
            table()
                .lookup_on(id, CachePlatform::AnthropicApi)
                .min_tokens,
            1024,
            "{id} on the first-party API"
        );
        assert_eq!(
            table().lookup_on(id, CachePlatform::Bedrock).min_tokens,
            4096,
            "{id} on Bedrock, which documents its own higher floor"
        );
    }
}

/// Opus 4.6 and Sonnet 4.6 take a one-hour TTL on the first-party API and five
/// minutes only on Bedrock. Requesting an hour where it is unsupported fails the
/// request outright, so this one errors loudly rather than silently — but it
/// still has to be right.
#[test]
fn the_extended_ttl_is_withheld_on_bedrock_where_aws_says_so() {
    for id in [
        "anthropic.claude-opus-4-6-v1",
        "anthropic.claude-sonnet-4-6",
    ] {
        assert!(
            table().lookup_on(id, CachePlatform::AnthropicApi).ttl_1h,
            "{id} accepts an hour on the first-party API"
        );
        assert!(
            !table().lookup_on(id, CachePlatform::Bedrock).ttl_1h,
            "{id}: Bedrock's support is contested between AWS's userguide and its \
             own samples matrix, so archon does not ask for the hour — see the \
             table comment"
        );
    }
}

/// Vertex serves Anthropic's own figures. Pinned rather than assumed, because
/// Google does withhold the extended TTL on several older Claude models — it
/// simply costs nothing to express today, since none of them carry a documented
/// one-hour TTL on any stack. The first model that genuinely diverges will fail
/// here, which is where the override belongs.
#[test]
fn vertex_and_the_first_party_api_agree_for_now() {
    for id in [
        "claude-opus-5",
        "anthropic.claude-sonnet-4-5-20250929-v1:0",
        "anthropic.claude-3-7-sonnet-20250219-v1:0",
    ] {
        assert_eq!(
            table().lookup_on(id, CachePlatform::Vertex),
            table().lookup_on(id, CachePlatform::AnthropicApi),
            "{id}: no Vertex divergence is documented for any model in the table"
        );
    }
}

/// GPT-5.6's limits are the same on OpenAI's API and on Bedrock. Only the
/// billing differs, and billing is not a threshold.
#[test]
fn gpt_5_6_resolves_the_same_on_both_stacks_that_serve_it() {
    let openai = table().lookup_on("gpt-5.6", CachePlatform::OpenAiApi);
    let bedrock = table().lookup_on("gpt-5.6", CachePlatform::Bedrock);

    assert_eq!(openai, bedrock);
    assert_eq!(openai.min_tokens, 1024);
    assert_eq!(openai.max_checkpoints, 4);
    assert!(
        !openai.ttl_1h,
        "the thirty-minute TTL is fixed, not requestable"
    );
}

/// A gateway resolves to the strictest figure any candidate stack imposes.
///
/// This is the £4.5k case. A LiteLLM proxy in front of Bedrock is configured as
/// `anthropic`, because that names the wire format it accepts — it translates to
/// Converse `cachePoint` itself. Archon cannot see through it, so taking
/// Anthropic's 1,024 at face value would put every Sonnet 4.5 checkpoint under
/// Bedrock's 4,096 floor, where it is discarded without an error and billed at
/// full price.
#[test]
fn an_unidentified_gateway_takes_the_highest_minimum_of_any_candidate_stack() {
    for id in [
        "anthropic.claude-sonnet-4-5-20250929-v1:0",
        "anthropic.claude-sonnet-5",
    ] {
        assert_eq!(
            table().lookup_on(id, CachePlatform::Unknown).min_tokens,
            4096,
            "{id}: a gateway may be fronting Bedrock, which needs 4,096"
        );
    }

    // Where the stacks agree there is nothing to raise, so a gateway must not
    // inflate the minimum beyond what any real endpoint asks for.
    assert_eq!(
        table()
            .lookup_on("claude-opus-5", CachePlatform::Unknown)
            .min_tokens,
        512,
        "no stack documents more than 512 for Opus 5"
    );
}

/// The TTL moves the opposite way under uncertainty. Too high a minimum merely
/// delays caching; an unsupported `1h` fails the request outright, so an hour is
/// requested only where every candidate stack allows one.
#[test]
fn an_unidentified_gateway_asks_for_an_hour_only_where_every_stack_allows_it() {
    for id in [
        "anthropic.claude-opus-4-6-v1",
        "anthropic.claude-sonnet-4-6",
    ] {
        assert!(
            table().lookup_on(id, CachePlatform::AnthropicApi).ttl_1h,
            "{id} accepts an hour on the first-party API"
        );
        assert!(
            !table().lookup_on(id, CachePlatform::Unknown).ttl_1h,
            "{id}: Bedrock caps it at five minutes, so a gateway must not ask"
        );
    }

    // Opus 5 has no such split, so uncertainty costs nothing here.
    assert!(
        table()
            .lookup_on("claude-opus-5", CachePlatform::Unknown)
            .ttl_1h
    );
}

/// The safe variant must be the one you get by forgetting. A provider that does
/// not override `cache_platform` should pay slightly more, never cache silently
/// into nothing.
#[test]
fn the_default_platform_is_the_unidentified_one() {
    assert_eq!(CachePlatform::default(), CachePlatform::Unknown);
}

/// Resolution consumes the per-platform overrides, so resolving twice must not
/// produce a third answer. Without this, a caller that resolved for Bedrock and
/// then passed the result through again would silently get Anthropic's figures
/// back on the second pass.
#[test]
fn resolving_a_second_time_changes_nothing() {
    for platform in [
        CachePlatform::AnthropicApi,
        CachePlatform::Bedrock,
        CachePlatform::Vertex,
        CachePlatform::OpenAiApi,
        CachePlatform::Unknown,
    ] {
        for id in ["anthropic.claude-sonnet-4-5-v1:0", "claude-opus-4-6"] {
            let once = table().lookup_on(id, platform);
            assert_eq!(once, once.on(platform), "{id} on {platform:?}");
            // Nor may re-resolving for a *different* platform reintroduce a
            // split that has already been applied.
            assert_eq!(
                once,
                once.on(CachePlatform::Bedrock),
                "{id} on {platform:?}, then re-resolved for Bedrock"
            );
        }
    }
}
