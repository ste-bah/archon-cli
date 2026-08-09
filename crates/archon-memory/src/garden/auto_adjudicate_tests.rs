//! Tests for the automatic-adjudication trigger.
//!
//! The trigger decides whether a session start spends an LLM round-trip on the
//! review band. Every one of these runs against the predicate alone: no
//! provider, no store, no clock.

use super::{GardenConfig, should_auto_adjudicate};

fn enabled(min_pairs: usize) -> GardenConfig {
    GardenConfig {
        auto_adjudicate_review_band: true,
        auto_adjudicate_min_pairs: min_pairs,
        ..GardenConfig::default()
    }
}

/// The default must never spend anything.
///
/// Automatic consolidation already runs unasked on the session-start path.
/// Adding a model call to it by default would charge every existing user for a
/// behaviour change they never opted into, and charge them in startup latency
/// where it is most visible.
#[test]
fn adjudication_is_off_by_default() {
    let config = GardenConfig::default();
    assert!(
        !config.auto_adjudicate_review_band,
        "automatic adjudication must default to off"
    );
    assert!(
        !should_auto_adjudicate(&config, 1_000),
        "the default must not adjudicate however large the band grows"
    );
}

/// A config written before this field existed must read as off.
///
/// The upgrade path is the one that matters here: an installed `config.toml`
/// has no `auto_adjudicate_review_band` key, and deserialising it must not turn
/// the feature on underneath someone. Driven through JSON because that is a
/// dependency this crate already has and `#[serde(default)]` fills a missing key
/// identically whatever the format.
#[test]
fn a_config_predating_the_field_deserialises_as_off() {
    let legacy = serde_json::json!({
        "auto_consolidate": true,
        "min_hours_between_runs": 24,
        "dedup_similarity_threshold": 0.92,
        "staleness_days": 30,
        "staleness_importance_floor": 0.3,
        "importance_decay_per_day": 0.01,
        "max_memories": 5000,
        "briefing_limit": 15,
    });
    let config: GardenConfig =
        serde_json::from_value(legacy).expect("legacy garden config must parse");

    assert!(!config.auto_adjudicate_review_band);
    assert_eq!(
        config.auto_adjudicate_min_pairs,
        super::default_auto_adjudicate_min_pairs(),
        "the threshold must fall back to its default, not to zero"
    );
}

/// Below the threshold the band keeps accumulating, which is the pre-existing
/// behaviour and costs nothing.
#[test]
fn below_the_threshold_does_not_trigger() {
    let config = enabled(10);
    assert!(!should_auto_adjudicate(&config, 9));
    assert!(!should_auto_adjudicate(&config, 1));
}

/// Exactly at the threshold fires.
///
/// Pinned as its own case because `>` and `>=` are indistinguishable everywhere
/// else, and the difference is a whole extra session of accumulation.
#[test]
fn at_and_above_the_threshold_triggers() {
    let config = enabled(10);
    assert!(should_auto_adjudicate(&config, 10));
    assert!(should_auto_adjudicate(&config, 47));
}

/// An empty band never justifies a call, whatever the threshold says.
///
/// `auto_adjudicate_min_pairs = 0` is the natural way to write "adjudicate
/// every time"; it must not also mean "ask the model about nothing" on every
/// session start of a clean store.
#[test]
fn an_empty_band_never_triggers() {
    assert!(!should_auto_adjudicate(&enabled(0), 0));
    assert!(!should_auto_adjudicate(&enabled(10), 0));
    assert!(
        should_auto_adjudicate(&enabled(0), 1),
        "a zero threshold must still fire on a non-empty band"
    );
}

/// Disabling wins over any amount of pending work.
#[test]
fn the_toggle_overrides_the_threshold() {
    let disabled = GardenConfig {
        auto_adjudicate_review_band: false,
        auto_adjudicate_min_pairs: 1,
        ..GardenConfig::default()
    };
    assert!(!should_auto_adjudicate(&disabled, 500));
}

/// The threshold must stay under the adjudicator's per-run cap.
///
/// `MAX_PAIRS_PER_RUN` in `src/command/garden_adjudicate.rs` is 20. If the
/// default threshold ever exceeded it, a run that fired at the threshold could
/// not clear the band it fired on, and the leftover would re-trigger on the very
/// next session start -- a call per launch, which is exactly what the threshold
/// exists to prevent.
#[test]
fn the_default_threshold_fits_inside_one_adjudication_batch() {
    assert!(
        super::default_auto_adjudicate_min_pairs() <= 20,
        "the default threshold must not exceed MAX_PAIRS_PER_RUN"
    );
}
