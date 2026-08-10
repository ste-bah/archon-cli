//! Tests for the bounds, the held-out guarantee, and the missing-outcome case.

use std::collections::BTreeMap;

use chrono::{TimeZone, Utc};

use super::policy::{
    MAX_DECILE_SHARE, MAX_PRIORITIZED_FRACTION, MAX_SURPRISE_WEIGHT, ReplayPolicy, TransitionKey,
    is_held_out, surprise_by_row_id,
};
use super::sampler::{ReplaySkipReason, plan_replay};
use crate::guardrail::WorldGuardrailOutcome;
use crate::schema::{WorldActionKind, WorldTraceRow};

fn key(session: &str, id: &str) -> TransitionKey {
    TransitionKey {
        transition_id: id.to_string(),
        session_id: session.to_string(),
        action_attempt_id: Some(format!("action-{id}")),
        created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
    }
}

/// `sessions` sessions of `per_session` transitions each.
fn corpus(sessions: usize, per_session: usize) -> Vec<TransitionKey> {
    (0..sessions)
        .flat_map(|session| {
            (0..per_session).map(move |index| {
                key(
                    &format!("session-{session:03}"),
                    &format!("t-{session:03}-{index:04}"),
                )
            })
        })
        .collect()
}

/// Surprise rising linearly across the corpus, so ranks are unambiguous.
fn graded_surprise(keys: &[TransitionKey]) -> BTreeMap<String, f32> {
    keys.iter()
        .enumerate()
        .map(|(index, key)| (key.transition_id.clone(), index as f32 * 0.01))
        .collect()
}

fn enabled() -> ReplayPolicy {
    ReplayPolicy {
        prioritized_enabled: true,
        ..ReplayPolicy::default()
    }
}

/// A batch smaller than the pool — otherwise "which transitions were chosen"
/// has only one answer and the bounds are untested.
fn enabled_batch(batch_size: usize) -> ReplayPolicy {
    ReplayPolicy {
        batch_size,
        ..enabled()
    }
}

// ---------------------------------------------------------------- held-out --

/// The partition is a function of session id and split version alone. Replacing
/// every surprise value — including making held-out sessions maximally
/// surprising — leaves it byte-identical.
#[test]
fn held_out_partition_does_not_move_with_surprise() {
    let keys = corpus(40, 10);
    let quiet: BTreeMap<String, f32> = keys
        .iter()
        .map(|key| (key.transition_id.clone(), 0.0))
        .collect();
    let loud: BTreeMap<String, f32> = keys
        .iter()
        .map(|key| (key.transition_id.clone(), 1_000_000.0))
        .collect();

    let first = plan_replay(&keys, &quiet, enabled(), keys.len());
    let second = plan_replay(&keys, &loud, enabled(), keys.len());

    assert_eq!(first.held_out_indices, second.held_out_indices);
    assert!(!first.held_out_indices.is_empty());
    assert_eq!(first.summary.held_out, second.summary.held_out);
}

/// No sampled index is ever a held-out index, even when the held-out sessions
/// carry every large surprise in the corpus.
#[test]
fn sampler_never_selects_a_held_out_transition() {
    let keys = corpus(40, 10);
    let held: Vec<&TransitionKey> = keys
        .iter()
        .filter(|key| is_held_out(&key.session_id, 0.2, 1))
        .collect();
    assert!(!held.is_empty(), "split must reserve something to protect");
    // Plant the entire priority signal inside the held-out partition.
    let surprise: BTreeMap<String, f32> = keys
        .iter()
        .map(|key| {
            let value = if is_held_out(&key.session_id, 0.2, 1) {
                999.0
            } else {
                0.001
            };
            (key.transition_id.clone(), value)
        })
        .collect();

    let plan = plan_replay(&keys, &surprise, enabled(), keys.len());

    let held_out: std::collections::BTreeSet<usize> =
        plan.held_out_indices.iter().copied().collect();
    for sample in &plan.selected {
        assert!(
            !held_out.contains(&sample.index),
            "held-out index {} was sampled",
            sample.index
        );
    }
    assert_eq!(plan.summary.held_out, held_out.len());
}

/// Splitting by session, not by transition: a session is wholly in or wholly
/// out, so no held-out target can appear inside a training context window.
#[test]
fn held_out_split_is_whole_sessions() {
    let keys = corpus(30, 8);
    let plan = plan_replay(&keys, &graded_surprise(&keys), enabled(), keys.len());
    let held: std::collections::BTreeSet<usize> = plan.held_out_indices.iter().copied().collect();

    for (index, key) in keys.iter().enumerate() {
        let session_held = is_held_out(&key.session_id, 0.2, 1);
        assert_eq!(held.contains(&index), session_held);
    }
}

/// The partition must not drift with the toolchain: FNV-1a over a fixed salt.
#[test]
fn held_out_split_is_stable_and_versioned() {
    assert!(!is_held_out("session-000", 0.0, 1));
    let first: Vec<bool> = (0..200)
        .map(|index| is_held_out(&format!("session-{index:03}"), 0.2, 1))
        .collect();
    let again: Vec<bool> = (0..200)
        .map(|index| is_held_out(&format!("session-{index:03}"), 0.2, 1))
        .collect();
    assert_eq!(first, again);

    let other_version: Vec<bool> = (0..200)
        .map(|index| is_held_out(&format!("session-{index:03}"), 0.2, 2))
        .collect();
    assert_ne!(first, other_version, "split_version must repartition");

    let held = first.iter().filter(|value| **value).count();
    assert!(
        (20..=60).contains(&held),
        "expected ~20% of 200, got {held}"
    );
}

// ------------------------------------------------------------------ bounds --

/// Bound 1 — the uniform floor. At most half of a batch is prioritised, so no
/// transition's selection probability can fall below `(1 - f)/n`.
#[test]
fn uniform_floor_caps_the_prioritized_share() {
    let keys = corpus(40, 10);
    let plan = plan_replay(
        &keys,
        &graded_surprise(&keys),
        enabled_batch(100),
        keys.len(),
    );
    let summary = &plan.summary;

    assert!(summary.selected > 0);
    let share = summary.prioritized_selected as f32 / summary.selected as f32;
    assert!(
        share <= MAX_PRIORITIZED_FRACTION + f32::EPSILON,
        "prioritized share {share} exceeded the floor"
    );
    assert!(summary.uniform_selected >= summary.prioritized_selected);
}

/// Bound 1's payoff — importance weights stay inside
/// `[1/((1-f) + f*C), 1/(1-f)]`, which at the defaults is `[0.4, 2.0]`.
#[test]
fn importance_weights_are_bounded_by_the_uniform_floor() {
    let keys = corpus(40, 10);
    let plan = plan_replay(
        &keys,
        &graded_surprise(&keys),
        enabled_batch(100),
        keys.len(),
    );

    let fraction = MAX_PRIORITIZED_FRACTION;
    let upper = 1.0 / (1.0 - fraction);
    let lower = 1.0 / ((1.0 - fraction) + fraction * MAX_SURPRISE_WEIGHT);
    for sample in &plan.selected {
        assert!(
            sample.importance_weight <= upper + 1e-4,
            "importance {} exceeded {upper}",
            sample.importance_weight
        );
        assert!(
            sample.importance_weight >= lower - 1e-4,
            "importance {} below {lower}",
            sample.importance_weight
        );
    }
    assert!(plan.summary.max_importance_weight <= upper + 1e-4);
    assert!(plan.summary.min_importance_weight >= lower - 1e-4);
}

/// Bound 2 — weights come from rank, not value, so one absurd outlier cannot
/// buy more than `MAX_SURPRISE_WEIGHT` times an ordinary transition's weight.
#[test]
fn one_outlier_cannot_dominate_the_weights() {
    let keys = corpus(40, 10);
    let mut surprise = graded_surprise(&keys);
    // A parse failure against a broken provider: five orders of magnitude out.
    let victim = keys[7].transition_id.clone();
    surprise.insert(victim.clone(), 1e9);

    let plan = plan_replay(&keys, &surprise, enabled_batch(100), keys.len());

    for sample in &plan.selected {
        assert!(
            (1.0..=MAX_SURPRISE_WEIGHT + 1e-4).contains(&sample.weight),
            "weight {} outside [1, {MAX_SURPRISE_WEIGHT}]",
            sample.weight
        );
    }
    let outlier_count = plan
        .selected
        .iter()
        .filter(|sample| sample.transition_id == victim)
        .count();
    assert!(outlier_count <= 1, "an outlier was drawn more than once");
}

/// Bound 3 — the roadmap's rollback trigger, enforced during the draw rather
/// than detected afterwards.
#[test]
fn no_priority_decile_supplies_more_than_its_share() {
    let keys = corpus(60, 10);
    let plan = plan_replay(
        &keys,
        &graded_surprise(&keys),
        enabled_batch(50),
        keys.len(),
    );

    let mut counts = [0_usize; 10];
    for sample in &plan.selected {
        counts[sample.priority_decile as usize] += 1;
    }
    let quota = ((plan.summary.selected as f32 * MAX_DECILE_SHARE).floor() as usize).max(1);
    for (decile, count) in counts.iter().enumerate() {
        assert!(
            *count <= quota,
            "decile {decile} supplied {count}, quota {quota}"
        );
    }
    assert!(plan.summary.max_decile_share <= MAX_DECILE_SHARE + 0.05);
}

/// The decile cap actually clamps rather than merely being satisfied: tightened
/// to 5%, the batch comes up short and the draw terminates instead of spinning
/// looking for a transition it is not allowed to take.
#[test]
fn a_binding_decile_cap_truncates_the_batch_rather_than_looping() {
    let keys = corpus(60, 10);
    let policy = ReplayPolicy {
        max_decile_share: 0.05,
        ..enabled_batch(50)
    };

    let plan = plan_replay(&keys, &graded_surprise(&keys), policy, keys.len());

    let quota = 2; // floor(50 * 0.05)
    let mut counts = [0_usize; 10];
    for sample in &plan.selected {
        counts[sample.priority_decile as usize] += 1;
    }
    for count in counts {
        assert!(count <= quota, "decile supplied {count}, quota {quota}");
    }
    assert_eq!(plan.summary.selected, 10 * quota);
    assert!(plan.summary.selected < plan.summary.requested_batch);
    assert!(plan.applied());
}

/// Config may only tighten the bounds; it can never widen one.
#[test]
fn policy_clamps_widened_config_values() {
    let wide = ReplayPolicy {
        prioritized_enabled: true,
        held_out_fraction: 0.95,
        batch_size: 0,
        prioritized_fraction: 1.0,
        max_surprise_weight: 500.0,
        max_decile_share: 1.0,
        seed: 1,
        split_version: 1,
    }
    .clamped();

    assert_eq!(wide.prioritized_fraction, MAX_PRIORITIZED_FRACTION);
    assert_eq!(wide.max_surprise_weight, MAX_SURPRISE_WEIGHT);
    assert_eq!(wide.max_decile_share, MAX_DECILE_SHARE);
    assert_eq!(wide.held_out_fraction, 0.5);
    assert_eq!(wide.batch_size, 1);

    let nonsense = ReplayPolicy {
        prioritized_fraction: f32::NAN,
        max_surprise_weight: f32::INFINITY,
        ..ReplayPolicy::default()
    }
    .clamped();
    assert_eq!(nonsense.prioritized_fraction, 0.0);
    assert_eq!(nonsense.max_surprise_weight, 1.0);
}

// ------------------------------------------------------------- refusal path --

/// The default is off, and off means the example set is untouched.
#[test]
fn disabled_policy_computes_a_plan_but_declines_to_apply() {
    let keys = corpus(40, 10);
    let plan = plan_replay(
        &keys,
        &graded_surprise(&keys),
        ReplayPolicy::default(),
        keys.len(),
    );

    assert!(!plan.applied());
    assert_eq!(plan.summary.skip_reason, Some(ReplaySkipReason::Disabled));
    // Still full shadow evidence.
    assert!(plan.summary.pool > 0);
    assert!(plan.summary.held_out > 0);
    assert!(plan.summary.with_surprise > 0);
    assert!(!plan.selected.is_empty());
}

/// A corpus that never recorded a surprise is not narrowed. Shrinking the
/// training set with no priority signal would cost data and buy nothing.
#[test]
fn no_surprise_signal_declines_to_apply() {
    let keys = corpus(40, 10);
    let plan = plan_replay(&keys, &BTreeMap::new(), enabled(), keys.len());

    assert!(!plan.applied());
    assert_eq!(
        plan.summary.skip_reason,
        Some(ReplaySkipReason::NoSurpriseSignal)
    );
    assert_eq!(plan.summary.with_surprise, 0);
}

/// A key list that disagrees with the example list is refused outright rather
/// than applied against mismatched positions.
#[test]
fn index_mismatch_refuses_the_plan() {
    let keys = corpus(40, 10);
    let plan = plan_replay(&keys, &graded_surprise(&keys), enabled(), keys.len() - 1);

    assert!(!plan.applied());
    assert_eq!(
        plan.summary.skip_reason,
        Some(ReplaySkipReason::IndexMismatch)
    );
}

/// Everything held out leaves nothing to sample, and that is reported, not
/// papered over.
#[test]
fn an_empty_pool_is_reported() {
    let keys = corpus(4, 5);
    let policy = ReplayPolicy {
        held_out_fraction: 0.5,
        ..enabled()
    };
    // Force the whole corpus into the held-out side by naming every session
    // one that the split reserves.
    let held_session = (0..500)
        .map(|index| format!("session-{index:03}"))
        .find(|session| is_held_out(session, 0.5, 1))
        .expect("some session hashes into the held-out side");
    let keys: Vec<TransitionKey> = keys
        .iter()
        .map(|key| TransitionKey {
            session_id: held_session.clone(),
            ..key.clone()
        })
        .collect();

    let plan = plan_replay(&keys, &graded_surprise(&keys), policy, keys.len());

    assert_eq!(plan.summary.pool, 0);
    assert!(!plan.applied());
    assert_eq!(plan.summary.skip_reason, Some(ReplaySkipReason::EmptyPool));
    assert!(plan.selected.is_empty());
}

#[path = "02_tests/join.rs"]
mod join;
