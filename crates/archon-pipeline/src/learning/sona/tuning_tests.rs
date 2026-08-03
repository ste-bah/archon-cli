use super::*;

fn observation(key: &str, pressure: f64, at: u64) -> TuningObservation {
    TuningObservation {
        parameter_key: key.to_string(),
        pressure,
        recorded_at: at,
    }
}

fn run_of(key: &str, pressure: f64, count: u32) -> Vec<TuningObservation> {
    (0..count)
        .map(|index| observation(key, pressure, u64::from(index) + 1))
        .collect()
}

/// The fail-closed rule: an unproven key reports no weight at all, which is
/// what makes the config layer hold the operator's default.
#[test]
fn a_key_under_the_evidence_threshold_reports_no_weight() {
    for count in 0..MIN_OBSERVATIONS {
        let tuner = SonaParameterTuner::from_history(
            "bug-hunt",
            &run_of("max_repair_iterations", 1.0, count),
        );
        let tuned = tuner.weight_for("max_repair_iterations");
        assert_eq!(
            tuned.weight, None,
            "{count} observation(s) must not produce a weight"
        );
        assert_eq!(tuned.observations, count);
    }
}

/// A key that has never been observed at all reports nothing, even when a
/// sibling key on the same class is fully proven.
#[test]
fn an_unobserved_key_reports_nothing_beside_a_proven_sibling() {
    let tuner =
        SonaParameterTuner::from_history("review", &run_of("max_repair_iterations", 1.0, 20));

    assert!(tuner.weight_for("max_repair_iterations").weight.is_some());
    let untouched = tuner.weight_for("verification_branch_timeout_secs");
    assert_eq!(untouched.weight, None);
    assert_eq!(untouched.observations, 0);
}

/// Direction: runs that exhausted the budget push the weight positive (grow).
#[test]
fn sustained_budget_exhaustion_produces_a_positive_weight() {
    let tuner = SonaParameterTuner::from_history(
        "bug-hunt",
        &run_of("max_repair_iterations", 1.0, MIN_OBSERVATIONS),
    );

    let weight = tuner
        .weight_for("max_repair_iterations")
        .weight
        .expect("threshold met");
    assert!(
        weight > 0.0,
        "exhausted budgets must ask to grow, got {weight}"
    );
}

/// Direction: runs that never touched the budget push the weight negative.
#[test]
fn sustained_slack_produces_a_negative_weight() {
    let tuner = SonaParameterTuner::from_history(
        "greenfield",
        &run_of("max_repair_iterations", 0.0, MIN_OBSERVATIONS),
    );

    let weight = tuner
        .weight_for("max_repair_iterations")
        .weight
        .expect("threshold met");
    assert!(weight < 0.0, "unused budgets may shrink, got {weight}");
}

/// Pressure 0.5 is the neutral point: half the budget used means no opinion.
#[test]
fn exactly_half_the_budget_moves_nothing() {
    let tuner =
        SonaParameterTuner::from_history("migration", &run_of("host_call_timeout_secs", 0.5, 20));

    let weight = tuner
        .weight_for("host_call_timeout_secs")
        .weight
        .expect("threshold met");
    assert!(
        weight.abs() < 1e-9,
        "neutral pressure moved the weight to {weight}"
    );
}

/// The learning rate must be slow enough that the number of runs needed to
/// move an iteration budget one step stays well above the evidence gate. If
/// this ever inverts, a budget can move on the same evidence that barely
/// qualified to be looked at.
#[test]
fn moving_an_iteration_budget_one_step_needs_far_more_than_the_evidence_gate() {
    // 3 -> 4 needs the value past 3.5, i.e. weight >= 1/3 at TUNING_SPAN 0.5.
    let required = 1.0 / 3.0;
    let mut needed = None;
    for count in 1..=200_u32 {
        let tuner = SonaParameterTuner::from_history(
            "bug-hunt",
            &run_of("max_repair_iterations", 1.0, count),
        );
        if tuner
            .weight_for("max_repair_iterations")
            .weight
            .is_some_and(|weight| weight >= required)
        {
            needed = Some(count);
            break;
        }
    }
    let needed = needed.expect("a saturated budget must eventually move one step");
    assert!(
        needed > MIN_OBSERVATIONS * 2,
        "one step took only {needed} runs; the gate is {MIN_OBSERVATIONS}"
    );
    assert!(
        needed < 40,
        "one step took {needed} runs, which is slow enough that the loop is closed only on paper"
    );
}

/// Routes must separate task classes, or a bug hunt's repair pressure ends up
/// setting a greenfield build's budget.
#[test]
fn task_classes_do_not_share_a_weight() {
    let bug_hunt =
        SonaParameterTuner::from_history("bug-hunt", &run_of("max_repair_iterations", 1.0, 20));
    let greenfield =
        SonaParameterTuner::from_history("greenfield", &run_of("max_repair_iterations", 0.0, 20));

    assert!(bug_hunt.weight_for("max_repair_iterations").weight.unwrap() > 0.0);
    assert!(
        greenfield
            .weight_for("max_repair_iterations")
            .weight
            .unwrap()
            < 0.0
    );
    assert_ne!(
        SonaParameterTuner::route("bug-hunt", "max_repair_iterations"),
        SonaParameterTuner::route("greenfield", "max_repair_iterations")
    );
}

/// A fresh key must be able to accept its first observations. Cosine
/// similarity against a zero vector reads as total divergence, so without the
/// empty-prior guard the learner would reject every key forever.
#[test]
fn the_first_observation_on_a_fresh_key_is_admitted() {
    let mut tuner = SonaParameterTuner::from_history("review", &[]);

    let outcome = tuner.admit(&[observation("max_repair_iterations", 1.0, 1)]);

    assert!(
        matches!(outcome, AdmissionOutcome::Admitted(_)),
        "{outcome:?}"
    );
}

/// Drift is wired, not merely available: a batch that reverses an established
/// model is rolled back and must not be persisted.
#[test]
fn a_reversing_batch_is_rejected_and_rolled_back() {
    let mut tuner = SonaParameterTuner::from_history(
        "bug-hunt",
        &run_of("verification_branch_timeout_secs", 1.0, 30),
    );
    let before = tuner
        .weight_for("verification_branch_timeout_secs")
        .weight
        .expect("threshold met");

    // Enough reversed pressure in one batch to flip the sign of the only
    // coordinate, which is a divergence of 2.0 — well past the reject line.
    let reversal: Vec<_> = (0..80)
        .map(|index| observation("verification_branch_timeout_secs", 0.0, 100 + index))
        .collect();
    let outcome = tuner.admit(&reversal);

    assert!(
        matches!(outcome, AdmissionOutcome::DriftRejected(_)),
        "{outcome:?}"
    );
    let after = tuner
        .weight_for("verification_branch_timeout_secs")
        .weight
        .expect("rollback must restore a usable weight");
    assert!(
        (after - before).abs() < 1e-12,
        "rollback left {after}, expected the checkpointed {before}"
    );
}

/// A rejected batch must not leave its observations counted, or the evidence
/// figure reported to the user would include outcomes the tuner discarded.
#[test]
fn a_rejected_batch_does_not_inflate_the_observation_count() {
    let mut tuner = SonaParameterTuner::from_history(
        "bug-hunt",
        &run_of("verification_branch_timeout_secs", 1.0, 30),
    );
    let before = tuner
        .weight_for("verification_branch_timeout_secs")
        .observations;

    let reversal: Vec<_> = (0..80)
        .map(|index| observation("verification_branch_timeout_secs", 0.0, 100 + index))
        .collect();
    assert!(matches!(
        tuner.admit(&reversal),
        AdmissionOutcome::DriftRejected(_)
    ));

    assert_eq!(
        tuner
            .weight_for("verification_branch_timeout_secs")
            .observations,
        before
    );
}

/// Replay order is by timestamp, not by the order rows came back from a store.
#[test]
fn replay_is_ordered_by_timestamp_not_by_input_order() {
    let forward = vec![
        observation("host_call_timeout_secs", 1.0, 1),
        observation("host_call_timeout_secs", 0.0, 2),
        observation("host_call_timeout_secs", 1.0, 3),
        observation("host_call_timeout_secs", 1.0, 4),
        observation("host_call_timeout_secs", 1.0, 5),
    ];
    let mut shuffled = forward.clone();
    shuffled.reverse();

    let a = SonaParameterTuner::from_history("review", &forward);
    let b = SonaParameterTuner::from_history("review", &shuffled);

    assert_eq!(
        a.weight_for("host_call_timeout_secs").weight,
        b.weight_for("host_call_timeout_secs").weight
    );
}

/// Out-of-range pressure is clamped rather than trusted; a caller that
/// computes a ratio wrong must not be able to saturate a weight in one run.
#[test]
fn pressure_outside_the_unit_range_is_clamped() {
    let wild = SonaParameterTuner::from_history(
        "review",
        &run_of("max_investigation_iterations", 25.0, 20),
    );
    let saturated = SonaParameterTuner::from_history(
        "review",
        &run_of("max_investigation_iterations", 1.0, 20),
    );

    assert_eq!(
        wild.weight_for("max_investigation_iterations").weight,
        saturated.weight_for("max_investigation_iterations").weight
    );
}
