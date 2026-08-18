//! Tests for per-message token attribution (#189 Phase 3).
//!
//! Kept beside `token_surface.rs` rather than inline so that file stays well
//! under the 500-line ceiling.

use super::*;

fn message(text: &str) -> serde_json::Value {
    serde_json::json!({"role": "user", "content": text})
}

fn messages(sizes: &[usize]) -> Vec<serde_json::Value> {
    sizes.iter().map(|n| message(&"x".repeat(*n))).collect()
}

#[test]
fn every_message_gets_a_node_in_order() {
    let surface = TokenSurface::build(&messages(&[10, 200, 30]), Calibration::default());

    let indices: Vec<usize> = surface.nodes().iter().map(|n| n.message_index).collect();
    assert_eq!(indices, vec![0, 1, 2]);
    assert_eq!(
        surface.total(),
        surface
            .nodes()
            .iter()
            .map(|n| n.estimated_tokens)
            .sum::<u64>()
    );
}

#[test]
fn an_empty_conversation_attributes_nothing() {
    let surface = TokenSurface::build(&[], Calibration::default());
    assert_eq!(surface.total(), 0);
    assert!(surface.top_contributors(5).is_empty());
    assert!(surface.nodes_covering(0.9).is_empty());
}

/// The question compaction could not previously ask.
#[test]
fn the_biggest_messages_are_identified() {
    let surface = TokenSurface::build(&messages(&[10, 4_000, 20, 8_000]), Calibration::default());

    let top = surface.top_contributors(2);

    assert_eq!(top.len(), 2);
    assert_eq!(top[0].message_index, 3);
    assert_eq!(top[1].message_index, 1);
}

/// Ranking must not reshuffle between refreshes, or a caller acting on "the
/// top contributor" acts on a different message each time.
#[test]
fn equal_messages_rank_in_a_stable_order() {
    let surface = TokenSurface::build(&messages(&[100, 100, 100]), Calibration::default());

    let first = surface.top_contributors(3);
    let second = surface.top_contributors(3);

    assert_eq!(first, second);
    let indices: Vec<usize> = first.iter().map(|n| n.message_index).collect();
    assert_eq!(indices, vec![0, 1, 2]);
}

/// One enormous message and many small ones should yield one entry, not a
/// fixed top-N that drags in messages worth nothing.
#[test]
fn covering_a_fraction_returns_the_fewest_messages_that_do_it() {
    let mut sizes = vec![100_000];
    sizes.extend(std::iter::repeat_n(10, 40));
    let surface = TokenSurface::build(&messages(&sizes), Calibration::default());

    let covering = surface.nodes_covering(0.9);

    assert_eq!(covering.len(), 1);
    assert_eq!(covering[0].message_index, 0);
}

#[test]
fn covering_everything_returns_every_message() {
    let surface = TokenSurface::build(&messages(&[10, 20, 30]), Calibration::default());
    assert_eq!(surface.nodes_covering(1.0).len(), 3);
}

#[test]
fn a_zero_fraction_selects_nothing() {
    let surface = TokenSurface::build(&messages(&[10, 20]), Calibration::default());
    assert!(surface.nodes_covering(0.0).is_empty());
}

/// The acceptance criterion: after a real provider response the summed surface
/// lands within 10% of the reported context size.
#[test]
fn calibration_brings_the_surface_within_ten_percent_of_the_provider_count() {
    let set = messages(&[5_000, 12_000, 800]);
    let raw = super::super::autocompact::estimate_messages_tokens(&set);
    // A tokenizer that packs ~30% more text per token than len/4 assumes.
    let actual = (raw as f64 * 0.7).round() as u64;
    let mut surface = TokenSurface::build(&set, Calibration::default());

    assert!(
        surface.reconcile(&set, actual),
        "a plausible ratio is accepted"
    );

    let error = (surface.total() as f64 - actual as f64).abs() / actual as f64;
    assert!(
        error <= 0.10,
        "within 10%: total={} actual={actual}",
        surface.total()
    );
}

/// The whole reason the factor is kept separately from the nodes: the anchor is
/// zeroed on compaction, and the surface must not fall back to a raw guess at
/// the exact moment the new size matters most.
#[test]
fn the_calibration_survives_a_rebuild_on_a_different_message_set() {
    let before = messages(&[9_000, 9_000]);
    let raw = super::super::autocompact::estimate_messages_tokens(&before);
    let mut surface = TokenSurface::build(&before, Calibration::default());
    surface.reconcile(&before, (raw as f64 * 0.7).round() as u64);
    let factor = surface.calibration().factor();

    // Compaction replaces the history wholesale and clears the anchor.
    let after = messages(&[400]);
    let rebuilt = TokenSurface::build(&after, surface.calibration());

    assert!(rebuilt.calibration().is_calibrated());
    assert!((rebuilt.calibration().factor() - factor).abs() < f64::EPSILON);
    let raw_after = super::super::autocompact::estimate_messages_tokens(&after);
    assert_ne!(
        rebuilt.total(),
        raw_after,
        "a rebuilt surface must stay corrected, not revert to len/4"
    );
}

/// A ratio this far out means the count and the message set describe different
/// things. Applying it would make every later estimate worse than no
/// correction at all.
#[test]
fn an_implausible_ratio_is_rejected_and_leaves_the_factor_alone() {
    let set = messages(&[1_000]);
    let mut calibration = Calibration::default();

    assert!(!calibration.observe(10_000, 5));
    assert!(!calibration.observe(10, 500_000));
    assert!(!calibration.is_calibrated());
    assert_eq!(
        TokenSurface::build(&set, calibration)
            .calibration()
            .factor(),
        1.0
    );
}

/// Turn one, after `/clear`, and immediately post-compaction all report zero.
/// Dividing by that would be a panic or an infinity.
#[test]
fn a_zero_usage_report_is_ignored() {
    let set = messages(&[1_000]);
    let mut surface = TokenSurface::build(&set, Calibration::default());

    assert!(!surface.reconcile(&set, 0));
    assert!(!surface.calibration().is_calibrated());
    assert!(surface.total() > 0, "the surface still attributes");
}

#[test]
fn reconciling_an_empty_set_cannot_divide_by_zero() {
    let mut surface = TokenSurface::build(&[], Calibration::default());
    assert!(!surface.reconcile(&[], 1_000));
    assert_eq!(surface.total(), 0);
}

/// The reset that motivated the whole design. `autocompact_agent.rs` and
/// `compaction.rs` clear `last_known_context_tokens` to 0 after compacting; if
/// the calibration lived alongside it, attribution would drop back to raw
/// `len / 4` at precisely the moment the new size matters most.
#[test]
fn attribution_stays_calibrated_across_the_post_compaction_anchor_reset() {
    use crate::agent::ConversationState;

    let mut state = ConversationState {
        messages: messages(&[9_000, 9_000]),
        ..ConversationState::default()
    };
    let raw = super::super::autocompact::estimate_messages_tokens(&state.messages);

    assert!(state.reconcile_token_surface((raw as f64 * 0.7).round() as u64));
    let factor = state.token_calibration.factor();

    // What compaction does: replace the history and zero the anchor.
    state.messages = messages(&[500]);
    state.last_known_context_tokens = 0;

    let surface = state.token_surface();
    assert!(surface.calibration().is_calibrated());
    assert!((surface.calibration().factor() - factor).abs() < f64::EPSILON);
    assert!(
        surface.total() > 0,
        "the surface must be populated from calibrated estimates, not zero"
    );
    assert_ne!(
        surface.total(),
        super::super::autocompact::estimate_messages_tokens(&state.messages),
        "and not from the uncorrected guess either"
    );
}

#[test]
fn a_fresh_state_attributes_before_any_provider_response() {
    use crate::agent::ConversationState;

    let state = ConversationState {
        messages: messages(&[1_000, 2_000]),
        ..ConversationState::default()
    };

    let surface = state.token_surface();
    assert_eq!(surface.nodes().len(), 2);
    assert!(surface.total() > 0);
    assert!(!surface.calibration().is_calibrated());
}

#[test]
fn requesting_more_contributors_than_exist_returns_what_there_is() {
    let surface = TokenSurface::build(&messages(&[10, 20]), Calibration::default());
    assert_eq!(surface.top_contributors(50).len(), 2);
}
