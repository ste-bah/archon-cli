use super::*;

const KNOB: TunableShapeKnob = TunableShapeKnob::ImplementationWaveFanoutWidth;

fn input(weight: Option<f64>) -> ShapeInput {
    ShapeInput {
        knob: KNOB,
        weight,
        observations: 12,
        drift_rolled_back: false,
    }
}

/// Constraint 1. The whole knob is inert until the learner says otherwise, and
/// "inert" has to mean the run's options are indistinguishable from a build
/// without this module — hence `applied_width() == None` rather than
/// `Some(cap)`.
#[test]
fn no_learner_at_all_yields_the_configured_cap_and_no_explicit_width() {
    let decision = decide_fanout_width(4, None);
    assert_eq!(decision.source, ShapeSource::Baseline);
    assert_eq!(decision.applied, 4);
    assert_eq!(decision.applied_width(), None);
    assert!(!decision.source.moved());
    assert!(!decision.source.noteworthy());
}

/// Constraint 1 again, on the path that matters more: the learner ran, found
/// too little evidence, and must report that rather than guessing.
#[test]
fn a_learner_with_no_proven_weight_holds_the_configured_cap() {
    let decision = decide_fanout_width(4, Some(&input(None)));
    assert_eq!(decision.source, ShapeSource::InsufficientEvidence);
    assert_eq!(decision.applied, 4);
    assert_eq!(decision.applied_width(), None);
}

#[test]
fn drift_rollback_is_reported_as_itself_not_as_missing_evidence() {
    let rolled_back = ShapeInput {
        drift_rolled_back: true,
        ..input(Some(1.0))
    };
    let decision = decide_fanout_width(8, Some(&rolled_back));
    assert_eq!(decision.source, ShapeSource::DriftRolledBack);
    assert_eq!(decision.applied, 8);
    assert!(decision.source.noteworthy(), "the operator must be told");
}

#[test]
fn a_non_finite_weight_is_a_learner_bug_and_falls_back_to_the_cap() {
    for weight in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let decision = decide_fanout_width(8, Some(&input(Some(weight))));
        assert_eq!(decision.source, ShapeSource::InsufficientEvidence);
        assert_eq!(decision.applied, 8);
    }
}

/// The sign convention. Positive pressure means contention was observed, which
/// means NARROW. Getting this backwards is the one mistake that would make the
/// knob dangerous, so it is asserted numerically rather than described.
#[test]
fn positive_pressure_narrows_and_a_saturated_weight_halves_the_width() {
    let decision = decide_fanout_width(8, Some(&input(Some(1.0))));
    assert_eq!(decision.source, ShapeSource::Learned);
    assert_eq!(
        decision.applied, 4,
        "span is half the baseline at weight 1.0"
    );
    assert_eq!(decision.applied_width(), Some(4));
}

/// The ratchet. No amount of contention-free evidence may widen a fan-out,
/// because no run ever tested a width above the operator's cap.
#[test]
fn no_negative_weight_can_widen_past_the_configured_cap() {
    for weight in [-0.001, -0.5, -1.0, -50.0] {
        let decision = decide_fanout_width(4, Some(&input(Some(weight))));
        assert_eq!(
            decision.applied, 4,
            "weight {weight} must not widen past the cap"
        );
        assert_eq!(decision.applied_width(), None);
    }
}

/// Constraint 3. Width 0 dispatches nothing, and a wave that dispatches nothing
/// walks the lifecycle into `blocked-loop-exhaustion` — the earned failure
/// handling firing on a defect the knob invented.
#[test]
fn the_floor_is_serial_dispatch_never_zero() {
    assert_eq!(KNOB.floor(), 1);
    // A cap of 1 with maximum pressure: halving rounds to 1, not 0.
    let decision = decide_fanout_width(1, Some(&input(Some(1.0))));
    assert!(decision.applied >= 1);
    // And an absurd baseline of 0 is still a runnable width.
    let degenerate = decide_fanout_width(0, Some(&input(Some(1.0))));
    assert_eq!(degenerate.applied, 1);
    assert_eq!(degenerate.baseline, 1);
}

#[test]
fn every_weight_over_every_cap_lands_inside_the_bounds() {
    for cap in [1u32, 2, 3, 4, 8, 16, 64] {
        for step in -20i32..=20 {
            let weight = f64::from(step) / 10.0;
            let decision = decide_fanout_width(cap, Some(&input(Some(weight))));
            assert!(
                (1..=cap.max(1)).contains(&decision.applied),
                "cap {cap} weight {weight} produced {}",
                decision.applied
            );
        }
    }
}

/// Constraint 4 of the module docs: the lint gate withdraws a proposal whole.
#[test]
fn a_refused_proposal_returns_to_the_cap_and_keeps_its_reason() {
    let mut decision = decide_fanout_width(8, Some(&input(Some(1.0))));
    assert_eq!(decision.applied, 4);
    decision.refuse("2 unsupported edge(s) in the declared dependency graph");
    assert_eq!(decision.applied, 8);
    assert_eq!(decision.applied_width(), None);
    assert_eq!(decision.source, ShapeSource::RefusedByDependencyGraph);
    assert!(decision.source.noteworthy());
    assert!(
        decision
            .refusal
            .as_deref()
            .is_some_and(|reason| reason.contains("unsupported edge")),
        "a refusal that does not say what refused it cannot be acted on"
    );
}

/// A lint may tighten and may never loosen — otherwise a lint finding could be
/// the reason a run got MORE concurrency, which inverts the whole gate.
#[test]
fn tightening_only_ever_narrows_and_never_crosses_the_floor() {
    let mut decision = decide_fanout_width(8, None);
    decision.tighten_to(3);
    assert_eq!(decision.applied, 3);
    decision.tighten_to(6);
    assert_eq!(decision.applied, 3, "a looser limit is ignored");
    decision.tighten_to(0);
    assert_eq!(decision.applied, 1, "the floor still binds");
}

/// Constraint 4 (SONA must not reach Phase 6) has a structural half in the
/// crate graph; this is the readable half. The two knob families must stay
/// disjoint so `the_tuner_can_only_move_timeouts_and_retry_counts` keeps
/// meaning what it says.
#[test]
fn shape_knobs_and_budget_parameters_share_no_route_key() {
    let budget: Vec<&str> = super::super::TunableGeneratedParameter::ALL
        .into_iter()
        .map(super::super::TunableGeneratedParameter::key)
        .collect();
    for knob in TunableShapeKnob::ALL {
        assert!(
            !budget.contains(&knob.key()),
            "shape knob '{}' collides with a Phase 7 budget route",
            knob.key()
        );
    }
}

/// The shape knob set is fenced the same way the budget set is: it may only
/// change how work is DISTRIBUTED, never whether work is accepted. A second
/// variant has to be checked against that sentence before this list changes.
#[test]
fn the_shape_tuner_can_only_move_fan_out_width() {
    let keys: Vec<&str> = TunableShapeKnob::ALL
        .into_iter()
        .map(TunableShapeKnob::key)
        .collect();
    assert_eq!(
        keys,
        ["implementation_wave_fanout_width"],
        "the shape knob set changed; every entry must be a distribution knob whose \
         dangerous direction is closed by a shipped clamp, never a stage that decides \
         whether work is accepted and never one that can remove a reviewer"
    );
}

#[test]
fn a_decision_round_trips_through_the_persisted_metadata_form() {
    let mut decision = decide_fanout_width(8, Some(&input(Some(0.6))));
    decision.refuse("declared dependency graph did not validate");
    let json = serde_json::to_string(&decision).expect("serializes");
    let back: ShapeDecision = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, decision);
    assert!(json.contains("implementation_wave_fanout_width"));
}

#[test]
fn an_unrefused_decision_does_not_carry_an_empty_refusal_field() {
    let decision = decide_fanout_width(8, None);
    let json = serde_json::to_string(&decision).expect("serializes");
    assert!(
        !json.contains("refusal"),
        "the common case must cost nothing in the run metadata: {json}"
    );
}
