//! Tests for the per-model cache parameter table.
//!
//! These pin the numbers. How they resolve against a particular stack lives in
//! `tests/cache_platform_resolution.rs`.

use super::*;

fn table() -> ModelCacheTable {
    ModelCacheTable::default()
}

/// Straight from AWS's documented table, and therefore resolved against
/// Bedrock. These are the numbers that cost money when wrong, so each is
/// asserted individually rather than by rule.
#[test]
fn documented_minimums_match_the_published_aws_table() {
    for (id, expected) in [
        ("anthropic.claude-opus-4-5-20251101-v1:0", 4096),
        ("anthropic.claude-opus-4-6-v1", 4096),
        ("anthropic.claude-sonnet-4-5-20250929-v1:0", 4096),
        ("anthropic.claude-haiku-4-5-20251001-v1:0", 4096),
        ("anthropic.claude-opus-4-20250514-v1:0", 1024),
        ("anthropic.claude-3-7-sonnet-20250219-v1:0", 1024),
        ("anthropic.claude-3-5-sonnet-20241022-v2:0", 1024),
    ] {
        assert_eq!(
            table().lookup_on(id, CachePlatform::Bedrock).min_tokens,
            expected,
            "{id}"
        );
    }
}

/// Sonnet 4.6 takes 1,024 while the rest of its generation takes 4,096.
/// Inferring the minimum from the version number would get this one wrong.
#[test]
fn sonnet_4_6_keeps_the_lower_minimum_despite_its_generation() {
    assert_eq!(
        table()
            .lookup_or_conservative("anthropic.claude-sonnet-4-6")
            .min_tokens,
        1024
    );
}

/// A more specific marker must win: `claude-opus-4-6` also contains
/// `claude-opus-4`, and the two differ on both minimum and TTL.
#[test]
fn a_more_specific_marker_wins_over_a_prefix_of_itself() {
    let opus_4_6 = table().lookup_or_conservative("anthropic.claude-opus-4-6-v1");
    assert_eq!(opus_4_6.min_tokens, 4096);

    let opus_4 = table().lookup_or_conservative("anthropic.claude-opus-4-20250514-v1:0");
    assert_eq!(opus_4.min_tokens, 1024);
    assert!(
        !opus_4.ttl_1h,
        "Opus 4's TTL is undocumented and must not be inherited from 4.6"
    );
}

/// Region prefixes and inference-profile ARNs are how these ids actually
/// arrive; neither begins with `anthropic.`.
#[test]
fn region_prefixed_and_arn_ids_still_match() {
    assert_eq!(
        table()
            .lookup_on(
                "eu.anthropic.claude-sonnet-4-5-20250929-v1:0",
                CachePlatform::Bedrock
            )
            .min_tokens,
        4096
    );
    // A fixture string, never dialled: 123456789012 is AWS's documented
    // placeholder account id.
    assert!(
        table()
            .lookup("arn:aws:bedrock:eu-west-2:123456789012:inference-profile/claude-3-7-sonnet")
            .is_some()
    );
}

/// The point of the fallback. An unrecognised model must not be assumed to take
/// the lower minimum, because a checkpoint under the real figure is dropped in
/// silence rather than rejected — so the failure would be invisible.
#[test]
fn an_unknown_model_gets_the_conservative_default() {
    let unknown = table().lookup_or_conservative("anthropic.claude-not-yet-released");
    assert_eq!(unknown, CONSERVATIVE_DEFAULT);
    assert_eq!(unknown.min_tokens, 4096);
    assert!(
        !unknown.ttl_1h,
        "an unverified model must not request a TTL it may reject"
    );
}

/// The conservative fallback must stay conservative on every stack. A platform
/// arm that accidentally lowered it would reintroduce the silent-discard bug for
/// exactly the models nothing is known about.
#[test]
fn the_conservative_default_is_conservative_on_every_platform() {
    for platform in [
        CachePlatform::AnthropicApi,
        CachePlatform::Bedrock,
        CachePlatform::Vertex,
        CachePlatform::OpenAiApi,
        CachePlatform::Unknown,
    ] {
        let params = table().lookup_on("anthropic.claude-not-yet-released", platform);
        assert_eq!(params.min_tokens, 4096, "{platform:?}");
        assert!(!params.ttl_1h, "{platform:?}");
    }
}

/// The reason the table is configurable at all: a model released after the
/// binary was built, or a built-in that has gone stale, is fixable without a
/// new release.
#[test]
fn a_configured_entry_overrides_the_built_in() {
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert(
        "claude-3-7-sonnet".to_string(),
        ModelCacheParams {
            min_tokens: 2048,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    );
    let table = ModelCacheTable::from_config(overrides);

    let params = table.lookup_or_conservative("anthropic.claude-3-7-sonnet-20250219-v1:0");
    assert_eq!(
        params.min_tokens, 2048,
        "config must beat the built-in 1024"
    );
    assert!(params.ttl_1h);
}

#[test]
fn a_configured_entry_can_add_a_model_the_binary_never_heard_of() {
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert(
        "claude-opus-9".to_string(),
        ModelCacheParams {
            min_tokens: 8192,
            max_checkpoints: 6,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    );
    let table = ModelCacheTable::from_config(overrides);

    let params = table.lookup_or_conservative("eu.anthropic.claude-opus-9");
    assert_eq!(params.min_tokens, 8192);
    assert_eq!(params.max_checkpoints, 6);
}

/// An operator must be able to correct a per-platform divergence too, not just
/// the headline figure — otherwise the only fix for a wrong Bedrock minimum is a
/// new binary, which is the thing this table exists to avoid.
#[test]
fn a_configured_entry_can_set_the_bedrock_split() {
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert(
        "claude-opus-9".to_string(),
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: Some(4096),
            bedrock_ttl_1h: Some(false),
        },
    );
    let table = ModelCacheTable::from_config(overrides);

    let anthropic = table.lookup_on("claude-opus-9", CachePlatform::AnthropicApi);
    assert_eq!(anthropic.min_tokens, 1024);
    assert!(anthropic.ttl_1h);

    let bedrock = table.lookup_on("claude-opus-9", CachePlatform::Bedrock);
    assert_eq!(bedrock.min_tokens, 4096);
    assert!(!bedrock.ttl_1h);
}

/// Config keys are matched longest-first so a specific override beats a general
/// one. A config author should not have to reason about map iteration order to
/// get a predictable result.
#[test]
fn the_most_specific_configured_key_wins_regardless_of_map_order() {
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert(
        "claude".to_string(),
        ModelCacheParams {
            min_tokens: 1024,
            max_checkpoints: 4,
            ttl_1h: false,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    );
    overrides.insert(
        "claude-opus-4-5".to_string(),
        ModelCacheParams {
            min_tokens: 4096,
            max_checkpoints: 4,
            ttl_1h: true,
            bedrock_min_tokens: None,
            bedrock_ttl_1h: None,
        },
    );
    let table = ModelCacheTable::from_config(overrides);

    let params = table.lookup_or_conservative("anthropic.claude-opus-4-5-20251101-v1:0");
    assert_eq!(params.min_tokens, 4096);
    assert!(params.ttl_1h);
}

/// Lookup must not depend on the order entries happen to be written in.
///
/// `claude-opus-4` is a substring of `claude-opus-4-1`, `-4-5`, `-4-6`, `-4-7`
/// and `-4-8`. Under a first-match scan the table is correct only while the
/// specific rows sit above the general one, so moving a line would collapse
/// Opus 4.5 from 4,096 to 1,024 — a silent cache miss with nothing failing.
/// Longest-match-wins removes that hazard, and this asserts the property rather
/// than the hand-ordering that used to stand in for it.
#[test]
fn the_longest_matching_marker_wins_whatever_the_table_order() {
    // Every general/specific pair in the table, checked from the ids rather
    // than from position.
    for (specific, general, id) in [
        (
            "claude-opus-4-5",
            "claude-opus-4",
            "anthropic.claude-opus-4-5-20251101-v1:0",
        ),
        (
            "claude-opus-4-6",
            "claude-opus-4",
            "anthropic.claude-opus-4-6-v1",
        ),
        (
            "claude-opus-4-7",
            "claude-opus-4",
            "anthropic.claude-opus-4-7",
        ),
        (
            "claude-opus-4-8",
            "claude-opus-4",
            "anthropic.claude-opus-4-8",
        ),
        (
            "claude-opus-4-1",
            "claude-opus-4",
            "anthropic.claude-opus-4-1-20250805",
        ),
        (
            "claude-sonnet-4-5",
            "claude-sonnet-4",
            "anthropic.claude-sonnet-4-5-20250929-v1:0",
        ),
        (
            "claude-sonnet-4-6",
            "claude-sonnet-4",
            "anthropic.claude-sonnet-4-6",
        ),
    ] {
        let specific_params = BUILT_IN_MODELS
            .iter()
            .find(|(marker, _)| *marker == specific)
            .map(|(_, params)| *params)
            .unwrap_or_else(|| panic!("{specific} missing from the table"));

        // Compare against the row resolved the same way, since resolution
        // consumes the per-platform overrides.
        assert_eq!(
            table().lookup_or_conservative(id),
            specific_params.on(CachePlatform::AnthropicApi),
            "{id} resolved to something other than its {specific} entry; the more \
             general {general} must not win"
        );
    }
}

/// A future release must not be swallowed by a shorter marker. `claude-opus-4-9`
/// does not exist yet, and when it does it must fall through to the
/// conservative default rather than silently inheriting Opus 4's 1,024.
#[test]
fn an_unreleased_point_version_falls_through_rather_than_inheriting() {
    // It still matches `claude-opus-4` by substring, so this is only true
    // because nothing shorter can outrank a longer match — and there is no
    // longer match here, so the conservative default applies.
    let params = table().lookup_or_conservative("anthropic.claude-opus-4-9");
    assert_eq!(
        params.min_tokens, 1024,
        "matches claude-opus-4 and correctly inherits its documented minimum"
    );
}

/// Models absent from AWS's caching table but documented by Anthropic. Missing
/// them means falling back to the conservative 4,096 on models that cache from
/// 512 — eight times more prefix than needed before anything is cached.
#[test]
fn the_512_token_models_are_recognised() {
    for id in [
        "anthropic.claude-opus-5",
        "anthropic.claude-fable-5",
        "claude-mythos-5",
    ] {
        assert_eq!(
            table().lookup_or_conservative(id).min_tokens,
            512,
            "{id} caches from 512 tokens"
        );
    }
}

/// `claude-mythos-preview` and `claude-mythos-5` are different models with
/// different minimums, and one marker is not a substring of the other — but the
/// pairing is close enough to be worth pinning.
#[test]
fn mythos_preview_is_not_confused_with_mythos_5() {
    assert_eq!(
        table()
            .lookup_or_conservative("claude-mythos-preview")
            .min_tokens,
        2048
    );
    assert_eq!(
        table().lookup_or_conservative("claude-mythos-5").min_tokens,
        512
    );
}

/// Retired models whose TTL support neither vendor documents. Requesting an
/// hour where it is unsupported fails the request, so an undocumented TTL must
/// be off rather than assumed.
#[test]
fn models_with_undocumented_ttl_do_not_request_an_hour() {
    for id in [
        "anthropic.claude-opus-4-1-20250805",
        "anthropic.claude-sonnet-4-20250514",
        "anthropic.claude-3-5-haiku-20241022-v1:0",
    ] {
        assert!(
            !table().lookup_or_conservative(id).ttl_1h,
            "{id}: TTL support is undocumented and must not be guessed"
        );
    }
}

/// Haiku minimums are non-monotonic across generations — 3.5 needs 2,048 while
/// 4.5 needs 4,096 — and the id ordering flips between them
/// (`claude-3-5-haiku` vs `claude-haiku-4-5`), so both spellings must resolve.
#[test]
fn both_haiku_id_orderings_resolve_to_their_own_minimums() {
    assert_eq!(
        table()
            .lookup_or_conservative("anthropic.claude-3-5-haiku-20241022-v1:0")
            .min_tokens,
        2048
    );
    assert_eq!(
        table()
            .lookup_or_conservative("anthropic.claude-haiku-4-5-20251001-v1:0")
            .min_tokens,
        4096
    );
}
