use chrono::{Duration, Utc};

use super::{RuleObservation, RuleOrigin, RuleRetirementPolicy, rule_retirement_candidates};

fn now() -> chrono::DateTime<chrono::Utc> {
    Utc::now()
}

/// A correction-derived rule that has gone completely quiet.
fn quiet_rule() -> RuleObservation {
    RuleObservation {
        rule_id: "rule-1".into(),
        rule_text: "check constraints before acting".into(),
        score: 12.0,
        origin: RuleOrigin::CorrectionDerived,
        created_at: now() - Duration::days(400),
        last_triggered: Some(now() - Duration::days(200)),
        supporting_corrections: 1,
        most_recent_correction: Some(now() - Duration::days(200)),
        in_prompt: true,
    }
}

#[test]
fn a_quiet_correction_derived_rule_is_proposed() {
    let candidates =
        rule_retirement_candidates(&[quiet_rule()], &RuleRetirementPolicy::default(), now());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].rule_id, "rule-1");
    assert_eq!(candidates[0].evidence.supporting_corrections, 1);
    assert_eq!(
        candidates[0].evidence.days_since_supporting_correction,
        Some(200)
    );
}

#[test]
fn a_user_defined_rule_is_never_proposed() {
    // Someone typed it. Its going quiet means the model stopped needing to be
    // told, which is the rule working.
    let rule = RuleObservation {
        origin: RuleOrigin::UserDefined,
        ..quiet_rule()
    };

    assert!(
        rule_retirement_candidates(&[rule], &RuleRetirementPolicy::default(), now()).is_empty()
    );
}

#[test]
fn a_shipped_default_is_never_proposed() {
    // Operator policy, not learned evidence. Removing one would be a config
    // change made by a background job.
    let rule = RuleObservation {
        origin: RuleOrigin::SystemDefault,
        ..quiet_rule()
    };

    assert!(
        rule_retirement_candidates(&[rule], &RuleRetirementPolicy::default(), now()).is_empty()
    );
}

#[test]
fn a_rule_that_is_not_in_the_prompt_is_not_proposed() {
    // It already occupies no slot, so retiring it changes nothing observable
    // while still spending a reviewer's decision.
    let rule = RuleObservation {
        in_prompt: false,
        ..quiet_rule()
    };

    assert!(
        rule_retirement_candidates(&[rule], &RuleRetirementPolicy::default(), now()).is_empty()
    );
}

#[test]
fn a_young_rule_is_never_proposed_however_quiet() {
    // A rule created last week has not had time to recur. Retiring it is the
    // system forgetting a lesson before it had a chance to be corroborated.
    let rule = RuleObservation {
        created_at: now() - Duration::days(3),
        ..quiet_rule()
    };

    assert!(
        rule_retirement_candidates(&[rule], &RuleRetirementPolicy::default(), now()).is_empty()
    );
}

#[test]
fn a_recent_supporting_correction_keeps_the_rule() {
    let rule = RuleObservation {
        most_recent_correction: Some(now() - Duration::days(2)),
        ..quiet_rule()
    };

    assert!(
        rule_retirement_candidates(&[rule], &RuleRetirementPolicy::default(), now()).is_empty(),
        "a rule the user is still hitting must not be retired"
    );
}

#[test]
fn recent_triggering_alone_keeps_the_rule() {
    // Both signals must be quiet. A rule still being matched is doing
    // something, even if no new correction was recorded.
    let rule = RuleObservation {
        last_triggered: Some(now() - Duration::days(1)),
        ..quiet_rule()
    };

    assert!(
        rule_retirement_candidates(&[rule], &RuleRetirementPolicy::default(), now()).is_empty()
    );
}

#[test]
fn a_heavily_corroborated_rule_is_not_proposed() {
    // Repeatedly earned. Silence is more likely to mean it is working than that
    // it expired.
    let rule = RuleObservation {
        supporting_corrections: 50,
        ..quiet_rule()
    };

    assert!(
        rule_retirement_candidates(&[rule], &RuleRetirementPolicy::default(), now()).is_empty()
    );
}

#[test]
fn a_rule_with_no_recorded_signal_at_all_is_quiet_not_unknown() {
    // For a rule older than the minimum age, absent timestamps are the
    // strongest form of silence. Treating them as "unknown, so keep" would make
    // the oldest and least-evidenced rules the ones that can never be retired.
    let rule = RuleObservation {
        last_triggered: None,
        most_recent_correction: None,
        ..quiet_rule()
    };

    let candidates = rule_retirement_candidates(&[rule], &RuleRetirementPolicy::default(), now());

    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].evidence.days_since_supporting_correction,
        None
    );
}

#[test]
fn the_evidence_records_the_policy_that_produced_it() {
    // A reviewer reading a proposal months later needs to know what "quiet"
    // meant at the time, not what it means now.
    let policy = RuleRetirementPolicy {
        quiet_days: 45,
        ..RuleRetirementPolicy::default()
    };

    let candidates = rule_retirement_candidates(&[quiet_rule()], &policy, now());

    assert_eq!(candidates[0].evidence.quiet_days, 45);
}

#[test]
fn nothing_is_proposed_from_an_empty_observation_set() {
    assert!(rule_retirement_candidates(&[], &RuleRetirementPolicy::default(), now()).is_empty());
}
