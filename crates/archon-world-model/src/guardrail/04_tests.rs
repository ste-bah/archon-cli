#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_score_treats_none_as_zero_and_applies_weights() {
        let score = risk_score(GuardrailRiskScores {
            predicted_retry: Some(1.0),
            predicted_high_cost: Some(1.0),
            ..GuardrailRiskScores::default()
        });

        assert_eq!(score, 0.75);
    }

    #[test]
    fn risk_tier_uses_inclusive_thresholds() {
        let policy = WorldGuardrailPolicyConfig::default();

        assert_eq!(risk_tier(0.85, &policy), WorldRiskTier::Critical);
        assert_eq!(risk_tier(0.70, &policy), WorldRiskTier::High);
        assert_eq!(risk_tier(0.45, &policy), WorldRiskTier::Medium);
        assert_eq!(risk_tier(0.44, &policy), WorldRiskTier::Low);
    }

    include!("03_action_classification_tests.rs");

    #[test]
    fn guarded_high_risk_coding_requires_verification() {
        let policy = WorldGuardrailPolicyConfig::default();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::CodingTask,
            GuardedActionKind::UserRequest,
            "build app",
            "build a Python app",
        );
        let context = WorldGuardrailPredictionContext::from_scores(
            RuntimeTaskClass::CodingChange,
            WorldGuardrailMode::Guarded,
            GuardrailRiskScores {
                predicted_verification_needed: Some(0.90),
                ..GuardrailRiskScores::default()
            },
            &policy,
        );

        let decision = decide_guardrail(&action, None, context, &policy);

        assert!(!decision.allowed_to_finalize);
        assert!(
            decision
                .required_actions
                .contains(&GuardrailRequiredAction::RunTests)
        );
        assert!(
            decision
                .required_actions
                .contains(&GuardrailRequiredAction::RunBuild)
        );
        assert!(
            decision
                .reason_codes
                .contains(&GuardrailReasonCode::PredictedVerificationNeededHigh)
        );
    }

    #[test]
    fn advisory_high_risk_warns_but_does_not_block() {
        let policy = WorldGuardrailPolicyConfig::default();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::InteractiveSession,
            GuardedActionKind::UserRequest,
            "code",
            "implement feature",
        );
        let context = WorldGuardrailPredictionContext::from_scores(
            RuntimeTaskClass::CodingChange,
            WorldGuardrailMode::Advisory,
            GuardrailRiskScores {
                predicted_verification_needed: Some(0.90),
                ..GuardrailRiskScores::default()
            },
            &policy,
        );

        let decision = decide_guardrail(&action, None, context, &policy);

        assert!(decision.allowed_to_finalize);
        assert!(decision.required_actions.is_empty());
    }

    #[test]
    fn finalization_requires_passed_verification_when_blocking() {
        let decision = WorldGuardrailDecision {
            mode: WorldGuardrailMode::Guarded,
            allowed_to_finalize: false,
            required_actions: vec![GuardrailRequiredAction::RunTests],
            ..Default::default()
        };

        assert!(!finalization_allowed(&decision, &[]));
        assert!(!finalization_allowed(
            &decision,
            &[VerificationOutcome {
                kind: VerificationKind::UnitTests,
                status: VerificationStatus::Failed,
                ..VerificationOutcome::default()
            }]
        ));
        assert!(finalization_allowed(
            &decision,
            &[VerificationOutcome {
                kind: VerificationKind::UnitTests,
                status: VerificationStatus::Passed,
                ..VerificationOutcome::default()
            }]
        ));
    }

    #[test]
    fn finalization_allows_explicitly_skipped_verification() {
        let decision = WorldGuardrailDecision {
            mode: WorldGuardrailMode::Guarded,
            allowed_to_finalize: false,
            required_actions: vec![GuardrailRequiredAction::RunTests],
            ..Default::default()
        };

        assert!(finalization_allowed(
            &decision,
            &[VerificationOutcome {
                kind: VerificationKind::UnitTests,
                status: VerificationStatus::Skipped,
                summary: "manual override: operator accepted the risk".into(),
                evidence_refs: vec!["manual_override:skip_verification".into()],
                ..VerificationOutcome::default()
            }]
        ));
    }

    #[test]
    fn finalization_rejects_bare_skipped_verification() {
        let decision = WorldGuardrailDecision {
            mode: WorldGuardrailMode::Guarded,
            allowed_to_finalize: false,
            required_actions: vec![GuardrailRequiredAction::RunTests],
            ..Default::default()
        };

        assert!(!finalization_allowed(
            &decision,
            &[VerificationOutcome {
                kind: VerificationKind::UnitTests,
                status: VerificationStatus::Skipped,
                summary: "skipped without explicit manual override".into(),
                ..VerificationOutcome::default()
            }]
        ));
    }

    #[test]
    fn finalization_accepts_explicit_verifier_record() {
        let decision = WorldGuardrailDecision {
            mode: WorldGuardrailMode::Guarded,
            allowed_to_finalize: false,
            required_actions: vec![GuardrailRequiredAction::RunVerifier],
            ..Default::default()
        };

        assert!(finalization_allowed(
            &decision,
            &[VerificationOutcome {
                kind: VerificationKind::Custom("verifier".into()),
                status: VerificationStatus::Skipped,
                summary: "manual verifier override".into(),
                evidence_refs: vec!["manual_override:approve".into()],
                ..VerificationOutcome::default()
            }]
        ));
    }

    #[test]
    fn finalization_uses_latest_outcome_for_each_required_kind() {
        let mut decision = WorldGuardrailDecision {
            mode: WorldGuardrailMode::Guarded,
            allowed_to_finalize: false,
            ..Default::default()
        };
        decision.required_actions = vec![
            GuardrailRequiredAction::RunTests,
            GuardrailRequiredAction::RunBuild,
        ];
        let old_failure = VerificationOutcome {
            kind: VerificationKind::UnitTests,
            status: VerificationStatus::Failed,
            created_at: Utc::now() - chrono::Duration::seconds(10),
            ..VerificationOutcome::default()
        };
        let test_pass = VerificationOutcome {
            kind: VerificationKind::UnitTests,
            status: VerificationStatus::Passed,
            created_at: Utc::now(),
            ..VerificationOutcome::default()
        };

        assert!(!finalization_allowed(
            &decision,
            &[old_failure.clone(), test_pass.clone()]
        ));
        assert!(finalization_allowed(
            &decision,
            &[
                old_failure,
                test_pass,
                VerificationOutcome {
                    kind: VerificationKind::Build,
                    status: VerificationStatus::Passed,
                    created_at: Utc::now(),
                    ..VerificationOutcome::default()
                }
            ]
        ));
    }

    #[test]
    fn finalization_breaks_equal_timestamp_ties_by_idempotency_key() {
        let mut decision = WorldGuardrailDecision::default();
        decision.mode = WorldGuardrailMode::Guarded;
        decision.allowed_to_finalize = false;
        decision.required_actions = vec![GuardrailRequiredAction::RunTests];
        let created_at = Utc::now();
        let passed = VerificationOutcome {
            kind: VerificationKind::UnitTests,
            status: VerificationStatus::Passed,
            idempotency_key: "verification:z-passed".into(),
            created_at,
            ..VerificationOutcome::default()
        };
        let failed = VerificationOutcome {
            kind: VerificationKind::UnitTests,
            status: VerificationStatus::Failed,
            idempotency_key: "verification:a-failed".into(),
            created_at,
            ..VerificationOutcome::default()
        };

        assert!(finalization_allowed(&decision, &[passed, failed]));
    }

    #[test]
    fn guardrail_overhead_budget_fails_open_without_double_decision() {
        let policy = WorldGuardrailPolicyConfig::default();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::CodingTask,
            GuardedActionKind::UserRequest,
            "build app",
            "build a Python app",
        );
        let context = WorldGuardrailPredictionContext::from_scores(
            RuntimeTaskClass::CodingChange,
            WorldGuardrailMode::Guarded,
            GuardrailRiskScores {
                predicted_verification_needed: Some(0.90),
                ..GuardrailRiskScores::default()
            },
            &policy,
        );
        let decision = decide_guardrail(&action, None, context, &policy);
        let decision_id = decision.decision_id.clone();
        let idempotency_key = decision.idempotency_key.clone();

        let fail_open = enforce_guardrail_overhead_budget(decision, 41, 40);

        assert_eq!(fail_open.decision_id, decision_id);
        assert_eq!(fail_open.idempotency_key, idempotency_key);
        assert!(fail_open.allowed_to_continue);
        assert!(fail_open.allowed_to_finalize);
        assert!(fail_open.required_actions.is_empty());
        assert!(
            fail_open
                .reason_codes
                .contains(&GuardrailReasonCode::GuardrailOverheadExceeded)
        );
        assert!(
            fail_open
                .reason_codes
                .contains(&GuardrailReasonCode::PredictedVerificationNeededHigh)
        );
    }

    #[test]
    fn structured_outcome_maps_to_labels_without_prose() {
        let outcome = WorldGuardrailOutcome {
            final_status: GuardrailFinalStatus::BlockedFailedVerification,
            verification_outcomes: vec![VerificationOutcome {
                status: VerificationStatus::Failed,
                ..VerificationOutcome::default()
            }],
            user_correction_observed: true,
            plan_drift_observed: true,
            retry_count: 1,
            ..WorldGuardrailOutcome::default()
        };

        let labels = labels_from_guardrail_outcome(&outcome);

        assert_eq!(labels.success, Some(false));
        assert!(labels.failure);
        assert!(labels.verification_needed);
        assert!(labels.user_correction);
        assert!(labels.plan_drift);
        assert!(labels.retry);
    }

    #[test]
    fn ledgers_append_and_load_guardrail_rows() {
        let temp = tempfile::tempdir().unwrap();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::InteractiveSession,
            GuardedActionKind::UserRequest,
            "build",
            "build",
        );
        append_guarded_action(temp.path(), &action).unwrap();
        let decision = WorldGuardrailDecision::unavailable(&action);
        append_guardrail_decision(temp.path(), &decision).unwrap();
        let verification = VerificationOutcome::passed(
            action.action_id.clone(),
            VerificationKind::UnitTests,
            "tests passed",
        );
        append_verification_outcome(temp.path(), &verification).unwrap();
        let outcome = WorldGuardrailOutcome::from_decision(
            &decision,
            RuntimeTaskClass::CodingChange,
            GuardrailFinalStatus::CompletedVerified,
            "done",
        );
        append_guardrail_outcome(temp.path(), &outcome).unwrap();

        let counts = guardrail_status_counts(temp.path());

        assert_eq!(counts.actions, 1);
        assert_eq!(counts.decisions, 1);
        assert_eq!(counts.verifications, 1);
        assert_eq!(counts.outcomes, 1);
        assert_eq!(counts.advisor_unavailable_decisions, 1);
    }

    #[test]
    fn guarded_action_loader_returns_latest_revision_per_action() {
        let temp = tempfile::tempdir().unwrap();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::InteractiveSession,
            GuardedActionKind::UserRequest,
            "build",
            "build",
        );
        let mut revised = action.clone();
        revised.idempotency_key = format!("{}:tool", action.idempotency_key);
        revised.verification_plan = vec![VerificationRequirement {
            requirement_id: "req-tests".into(),
            kind: VerificationKind::UnitTests,
            command_hint: Some("cargo test".into()),
            applies_to: vec![action.action_id.clone()],
            required_for_final: true,
        }];

        let ledger = append_guarded_action(temp.path(), &action).unwrap();
        append_guarded_action(temp.path(), &revised).unwrap();

        assert_eq!(std::fs::read_to_string(ledger).unwrap().lines().count(), 2);
        let loaded = load_guarded_actions(temp.path()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].action_id, action.action_id);
        assert_eq!(loaded[0].verification_plan, revised.verification_plan);
    }

    #[test]
    fn guardrail_revision_persists_action_and_decision_together() {
        let temp = tempfile::tempdir().unwrap();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::InteractiveSession,
            GuardedActionKind::UserRequest,
            "build",
            "build",
        );
        let original_decision = WorldGuardrailDecision::unavailable(&action);
        append_guardrail_decision(temp.path(), &original_decision).unwrap();
        let mut revised = action.clone();
        revised.verification_plan = vec![VerificationRequirement::new(VerificationKind::UnitTests)];
        let mut decision = WorldGuardrailDecision::unavailable(&revised);
        decision.required_actions = vec![GuardrailRequiredAction::RunTests];

        let ledger = append_guardrail_revision(
            temp.path(),
            revised.clone(),
            decision.clone(),
            "revision-1".into(),
        )
        .unwrap();

        assert_eq!(std::fs::read_to_string(ledger).unwrap().lines().count(), 1);
        assert_eq!(load_guarded_actions(temp.path()).unwrap(), vec![revised]);
        assert_eq!(
            load_guardrail_decisions(temp.path()).unwrap(),
            vec![decision]
        );
    }

    #[test]
    fn guardrail_ledgers_skip_duplicate_idempotency_keys() {
        let temp = tempfile::tempdir().unwrap();
        let action = WorldGuardedAction::new(
            "s1",
            WorldAdvisorSurface::CodingTask,
            GuardedActionKind::UserRequest,
            "build",
            "build",
        );
        let decision = WorldGuardrailDecision::unavailable(&action);

        append_guarded_action(temp.path(), &action).unwrap();
        append_guarded_action(temp.path(), &action).unwrap();
        append_guardrail_decision(temp.path(), &decision).unwrap();
        append_guardrail_decision(temp.path(), &decision).unwrap();

        let counts = guardrail_status_counts(temp.path());

        assert_eq!(counts.actions, 1);
        assert_eq!(counts.decisions, 1);
    }
}
