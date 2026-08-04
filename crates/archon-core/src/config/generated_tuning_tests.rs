use super::*;
use crate::config::validate;

fn input(
    parameter: TunableGeneratedParameter,
    weight: Option<f64>,
    observations: u32,
) -> GeneratedTuningInput {
    GeneratedTuningInput {
        parameter,
        weight,
        observations,
        drift_rolled_back: false,
    }
}

fn decision(
    decisions: &[GeneratedTuningDecision],
    parameter: TunableGeneratedParameter,
) -> GeneratedTuningDecision {
    *decisions
        .iter()
        .find(|decision| decision.parameter == parameter)
        .expect("every parameter must be reported")
}

/// Constraint 1: a key with no outcome evidence holds its configured default.
#[test]
fn no_evidence_holds_the_configured_default_for_every_parameter() {
    let baseline = GeneratedWorkflowConfig::default();
    let inputs: Vec<_> = TunableGeneratedParameter::ALL
        .into_iter()
        .map(|parameter| input(parameter, None, 0))
        .collect();

    let (tuned, decisions) = apply_generated_tuning(&baseline, &inputs);

    assert_eq!(tuned.max_repair_iterations, baseline.max_repair_iterations);
    assert_eq!(
        tuned.max_investigation_iterations,
        baseline.max_investigation_iterations
    );
    assert_eq!(
        tuned.verification_branch_timeout_secs,
        baseline.verification_branch_timeout_secs
    );
    assert_eq!(
        tuned.host_call_timeout_secs,
        baseline.host_call_timeout_secs
    );
    for decision in &decisions {
        assert_eq!(decision.source, TuningSource::InsufficientEvidence);
        assert!(!decision.source.moved());
    }
}

/// An absent proposal is not the same as a zero weight, but both hold.
#[test]
fn an_absent_proposal_and_a_zero_weight_both_hold_the_default() {
    let baseline = GeneratedWorkflowConfig::default();

    let (absent, absent_decisions) = apply_generated_tuning(&baseline, &[]);
    let (zero, zero_decisions) = apply_generated_tuning(
        &baseline,
        &[input(
            TunableGeneratedParameter::MaxRepairIterations,
            Some(0.0),
            50,
        )],
    );

    assert_eq!(absent.max_repair_iterations, 3);
    assert_eq!(zero.max_repair_iterations, 3);
    assert_eq!(
        decision(
            &absent_decisions,
            TunableGeneratedParameter::MaxRepairIterations
        )
        .source,
        TuningSource::Baseline
    );
    assert_eq!(
        decision(
            &zero_decisions,
            TunableGeneratedParameter::MaxRepairIterations
        )
        .source,
        TuningSource::Learned,
        "evidence that says 'do not move' is still evidence and must be reported as learned"
    );
}

/// Constraint 2: the floors bind. A saturated negative weight cannot take any
/// parameter below the value that its comment says breaks things.
#[test]
fn a_saturated_negative_weight_stops_at_every_floor() {
    let baseline = GeneratedWorkflowConfig::default();
    let inputs: Vec<_> = TunableGeneratedParameter::ALL
        .into_iter()
        .map(|parameter| input(parameter, Some(-1.0), 500))
        .collect();

    let (tuned, decisions) = apply_generated_tuning(&baseline, &inputs);

    assert_eq!(tuned.max_repair_iterations, 2);
    assert_eq!(tuned.max_investigation_iterations, 2);
    assert_eq!(tuned.verification_branch_timeout_secs, 7_200);
    assert_eq!(tuned.host_call_timeout_secs, 3_600);
    for parameter in TunableGeneratedParameter::ALL {
        assert!(
            decision(&decisions, parameter).applied >= parameter.floor(),
            "{} fell below its floor",
            parameter.key()
        );
    }
}

/// The 1200s incident, restated as a bound. No sequence of weights may put a
/// verification branch under two hours.
#[test]
fn no_weight_can_starve_a_verification_branch_below_two_hours() {
    for weight in [-1.0, -0.99, -0.5, -1e9, f64::NEG_INFINITY] {
        let (tuned, _) = apply_generated_tuning(
            &GeneratedWorkflowConfig::default(),
            &[input(
                TunableGeneratedParameter::VerificationBranchTimeoutSecs,
                Some(weight),
                1_000,
            )],
        );
        assert!(
            tuned.verification_branch_timeout_secs >= 7_200,
            "weight {weight} starved the verifier to {}",
            tuned.verification_branch_timeout_secs
        );
    }
}

/// A saturated positive weight stops at every ceiling.
#[test]
fn a_saturated_positive_weight_stops_at_every_ceiling() {
    let baseline = GeneratedWorkflowConfig::default();
    let inputs: Vec<_> = TunableGeneratedParameter::ALL
        .into_iter()
        .map(|parameter| input(parameter, Some(1.0), 500))
        .collect();

    let (tuned, decisions) = apply_generated_tuning(&baseline, &inputs);

    assert_eq!(tuned.max_repair_iterations, 5);
    assert_eq!(tuned.verification_branch_timeout_secs, 21_600);
    assert_eq!(tuned.host_call_timeout_secs, 10_800);
    for parameter in TunableGeneratedParameter::ALL {
        assert!(decision(&decisions, parameter).applied <= parameter.ceiling());
    }
}

/// Constraint 3 of the bounds contract: whatever the learner proposes, the
/// tuned config must still pass the same validation an operator's file does.
#[test]
fn every_extreme_weight_still_produces_a_validating_config() {
    for weight in [-1.0, -0.3, 0.0, 0.3, 1.0, f64::INFINITY, f64::NAN] {
        let inputs: Vec<_> = TunableGeneratedParameter::ALL
            .into_iter()
            .map(|parameter| input(parameter, Some(weight), 900))
            .collect();
        let (tuned, _) = apply_generated_tuning(&GeneratedWorkflowConfig::default(), &inputs);
        let mut config = crate::config::ArchonConfig::default();
        config.workflow.generated = tuned;
        validate(&config)
            .unwrap_or_else(|err| panic!("weight {weight} produced an invalid config: {err}"));
    }
}

/// A non-finite weight is a learner bug; the answer is the operator's value.
#[test]
fn a_non_finite_weight_is_treated_as_no_evidence() {
    for weight in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let (tuned, decisions) = apply_generated_tuning(
            &GeneratedWorkflowConfig::default(),
            &[input(
                TunableGeneratedParameter::MaxRepairIterations,
                Some(weight),
                900,
            )],
        );
        assert_eq!(tuned.max_repair_iterations, 3, "weight {weight}");
        assert_eq!(
            decision(&decisions, TunableGeneratedParameter::MaxRepairIterations).source,
            TuningSource::InsufficientEvidence
        );
    }
}

/// Constraint 4's visible half: a rollback is reported as a rollback, not as
/// an indistinguishable "no evidence".
#[test]
fn a_drift_rollback_is_reported_distinctly_and_uses_the_baseline() {
    let (tuned, decisions) = apply_generated_tuning(
        &GeneratedWorkflowConfig::default(),
        &[GeneratedTuningInput {
            parameter: TunableGeneratedParameter::MaxRepairIterations,
            weight: Some(1.0),
            observations: 400,
            drift_rolled_back: true,
        }],
    );

    assert_eq!(tuned.max_repair_iterations, 3);
    let decision = decision(&decisions, TunableGeneratedParameter::MaxRepairIterations);
    assert_eq!(decision.source, TuningSource::DriftRolledBack);
    assert!(
        decision.source.moved(),
        "a rollback must be visible in the run report"
    );
}

/// The cross-parameter invariant: a verifier never gets less clock than the
/// host call it inspects, even when each value is individually legal.
#[test]
fn verification_is_raised_when_a_learned_host_call_would_outlast_it() {
    let baseline = GeneratedWorkflowConfig {
        verification_branch_timeout_secs: 7_200,
        host_call_timeout_secs: 7_200,
        ..GeneratedWorkflowConfig::default()
    };

    let (tuned, decisions) = apply_generated_tuning(
        &baseline,
        &[
            input(
                TunableGeneratedParameter::VerificationBranchTimeoutSecs,
                Some(-1.0),
                600,
            ),
            input(
                TunableGeneratedParameter::HostCallTimeoutSecs,
                Some(1.0),
                600,
            ),
        ],
    );

    assert_eq!(tuned.host_call_timeout_secs, 10_800);
    assert_eq!(
        tuned.verification_branch_timeout_secs, 10_800,
        "verification must never be shorter than the host call it verifies"
    );
    assert_eq!(
        decision(
            &decisions,
            TunableGeneratedParameter::VerificationBranchTimeoutSecs
        )
        .source,
        TuningSource::RaisedByVerificationInvariant
    );
}

/// The invariant must not rewrite an operator's own configuration when the
/// learner had nothing to say.
#[test]
fn the_invariant_does_not_touch_an_untuned_operator_config() {
    let baseline = GeneratedWorkflowConfig {
        verification_branch_timeout_secs: 600,
        host_call_timeout_secs: 7_200,
        ..GeneratedWorkflowConfig::default()
    };

    let (tuned, _) = apply_generated_tuning(&baseline, &[]);

    assert_eq!(tuned.verification_branch_timeout_secs, 600);
    assert_eq!(tuned.host_call_timeout_secs, 7_200);
}

/// Bounds must stay a strict subset of the validated range, or a learned value
/// could produce a config an operator could not have written.
#[test]
fn every_bound_sits_inside_the_validated_range() {
    for parameter in TunableGeneratedParameter::ALL {
        assert!(
            parameter.floor() < parameter.ceiling(),
            "{}",
            parameter.key()
        );
        let (validated_low, validated_high) = match parameter {
            TunableGeneratedParameter::MaxRepairIterations
            | TunableGeneratedParameter::MaxInvestigationIterations => (1, 8),
            TunableGeneratedParameter::VerificationBranchTimeoutSecs
            | TunableGeneratedParameter::HostCallTimeoutSecs => (300, 86_400),
        };
        assert!(
            parameter.floor() >= validated_low && parameter.ceiling() <= validated_high,
            "{} bounds escape the validated range",
            parameter.key()
        );
    }
}
