//! Surprise-join, determinism and distribution tests.
//!
//! Split from the parent test module to stay under the file-size gate; the
//! helpers and imports it shares live there and are reached through super.

use std::collections::BTreeMap;

use super::super::policy::{ReplayPolicy, surprise_by_row_id};
use super::super::sampler::plan_replay;
use super::{corpus, enabled, enabled_batch, graded_surprise};
use crate::guardrail::WorldGuardrailOutcome;
use crate::schema::{WorldActionKind, WorldTraceRow};

// --------------------------------------------------------- surprise inputs --

/// A transition with no recorded outcome keeps the baseline weight: absence of
/// a scored prediction is not evidence of a small error, and zero weight would
/// silently delete every unscored transition from training.
#[test]
fn transitions_without_an_outcome_keep_the_baseline_weight() {
    let keys = corpus(40, 10);
    // Only every third transition was ever scored.
    let surprise: BTreeMap<String, f32> = keys
        .iter()
        .enumerate()
        .filter(|(index, _)| index % 3 == 0)
        .map(|(index, key)| (key.transition_id.clone(), index as f32 * 0.01))
        .collect();

    let plan = plan_replay(&keys, &surprise, enabled(), keys.len());

    let unscored: Vec<_> = plan
        .selected
        .iter()
        .filter(|sample| !surprise.contains_key(&sample.transition_id))
        .collect();
    assert!(!unscored.is_empty(), "unscored transitions must survive");
    for sample in unscored {
        assert!((sample.weight - 1.0).abs() < 1e-6, "{}", sample.weight);
    }
}

/// The join is action-attempt identity, and it refuses to invent values:
/// missing, non-finite and negative surprises contribute nothing.
#[test]
fn surprise_join_skips_rows_it_cannot_anchor() {
    let mut scored = WorldTraceRow::new("s", WorldActionKind::ToolCall).with_row_id("row-scored");
    scored.action_attempt_id = Some("action-1".into());
    let mut unscored =
        WorldTraceRow::new("s", WorldActionKind::ToolCall).with_row_id("row-unscored");
    unscored.action_attempt_id = Some("action-2".into());
    let mut nan = WorldTraceRow::new("s", WorldActionKind::ToolCall).with_row_id("row-nan");
    nan.action_attempt_id = Some("action-3".into());
    let mut negative =
        WorldTraceRow::new("s", WorldActionKind::ToolCall).with_row_id("row-negative");
    negative.action_attempt_id = Some("action-4".into());
    let detached = WorldTraceRow::new("s", WorldActionKind::ToolCall).with_row_id("row-detached");

    let outcome = |action: &str, surprise: Option<f32>| WorldGuardrailOutcome {
        action_id: action.to_string(),
        latent_surprise: surprise,
        ..WorldGuardrailOutcome::default()
    };
    let outcomes = vec![
        outcome("action-1", Some(0.75)),
        outcome("action-2", None),
        outcome("action-3", Some(f32::NAN)),
        outcome("action-4", Some(-1.0)),
        // No trace row anchors this one.
        outcome("action-9", Some(0.5)),
    ];

    let joined = surprise_by_row_id(&[scored, unscored, nan, negative, detached], &outcomes);

    assert_eq!(joined.len(), 1);
    assert_eq!(joined.get("row-scored"), Some(&0.75));
}

/// The last finalisation of an action is the current one.
#[test]
fn surprise_join_takes_the_latest_outcome_for_an_action() {
    let mut row = WorldTraceRow::new("s", WorldActionKind::ToolCall).with_row_id("row-1");
    row.action_attempt_id = Some("action-1".into());
    let outcomes = vec![
        WorldGuardrailOutcome {
            action_id: "action-1".into(),
            latent_surprise: Some(0.1),
            ..WorldGuardrailOutcome::default()
        },
        WorldGuardrailOutcome {
            action_id: "action-1".into(),
            latent_surprise: Some(0.9),
            ..WorldGuardrailOutcome::default()
        },
    ];

    let joined = surprise_by_row_id(&[row], &outcomes);

    assert_eq!(joined.get("row-1"), Some(&0.9));
}

// ------------------------------------------------------------ determinism --

/// A plan is reproducible from `(seed, split_version)`, which is what makes a
/// matched baseline/canary pair replayable.
#[test]
fn plans_are_deterministic_and_reseedable() {
    let keys = corpus(40, 10);
    let surprise = graded_surprise(&keys);

    let first = plan_replay(&keys, &surprise, enabled_batch(100), keys.len());
    let again = plan_replay(&keys, &surprise, enabled_batch(100), keys.len());
    assert_eq!(first.selected_indices(), again.selected_indices());

    let reseeded = plan_replay(
        &keys,
        &surprise,
        ReplayPolicy {
            seed: 99,
            ..enabled_batch(100)
        },
        keys.len(),
    );
    assert_ne!(first.selected_indices(), reseeded.selected_indices());
    // A different seed draws a different batch but cannot move the partition.
    assert_eq!(first.held_out_indices, reseeded.held_out_indices);
}

/// Selected positions are unique — no transition is trained on twice inside a
/// batch just because it was surprising.
#[test]
fn a_batch_contains_no_duplicates() {
    let keys = corpus(40, 10);
    let plan = plan_replay(
        &keys,
        &graded_surprise(&keys),
        enabled_batch(100),
        keys.len(),
    );

    let unique: std::collections::BTreeSet<usize> =
        plan.selected.iter().map(|sample| sample.index).collect();
    assert_eq!(unique.len(), plan.selected.len());
}

/// Surprise prioritisation actually does something: with the flag on, the
/// selected batch is skewed toward the high-surprise half of the pool relative
/// to the uniform expectation.
#[test]
fn prioritized_batches_over_sample_the_surprising_half() {
    let keys = corpus(60, 10);
    let surprise = graded_surprise(&keys);
    let plan = plan_replay(&keys, &surprise, enabled_batch(100), keys.len());

    let top_half = plan
        .selected
        .iter()
        .filter(|sample| sample.priority_decile < 5)
        .count() as f32
        / plan.summary.selected as f32;
    assert!(
        top_half > 0.5,
        "prioritized batch was not skewed: {top_half}"
    );
    // …and the skew is bounded, not a takeover.
    assert!(top_half < 0.8, "prioritized batch took over: {top_half}");
}
