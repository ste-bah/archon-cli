    use super::*;

    #[test]
    fn policy_from_config_maps_modes_and_overhead() {
        let mut config = archon_core::config::ArchonConfig::default();
        config.learning.world_model.guardrails.interactive_mode = "guarded".into();
        config
            .learning
            .world_model
            .guardrails
            .max_guardrail_overhead_ms = 41;

        let policy = policy_from_config(&config);

        assert_eq!(
            policy.interactive_mode,
            archon_world_model::WorldGuardrailMode::Guarded
        );
        assert_eq!(policy.max_guardrail_overhead_ms, 41);
    }

    #[test]
    fn guardrail_scores_for_prediction_prefers_learned_auxiliary_scores() {
        let mut prediction = archon_world_model::WorldPrediction::new("model-1", "next state");
        prediction.guardrail_scores = Some(archon_world_model::GuardrailRiskScores {
            predicted_verification_needed: Some(0.05),
            predicted_user_correction: Some(0.88),
            ..archon_world_model::GuardrailRiskScores::default()
        });

        let scores = guardrail_scores_for_prediction(
            archon_world_model::RuntimeTaskClass::CodingChange,
            Some(&prediction),
        );

        assert_eq!(scores.predicted_verification_needed, Some(0.05));
        assert_eq!(scores.predicted_user_correction, Some(0.88));
    }

    #[test]
    fn guardrail_scores_for_prediction_falls_back_to_task_defaults() {
        let scores = guardrail_scores_for_prediction(
            archon_world_model::RuntimeTaskClass::CodingChange,
            None,
        );

        assert_eq!(scores.predicted_verification_needed, Some(0.72));
    }

    #[test]
    fn learned_low_scores_change_guarded_coding_decision_from_block_to_allow() {
        let policy = archon_world_model::WorldGuardrailPolicyConfig::default();
        let action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "implement feature",
            "implement feature",
        );
        let default_context = archon_world_model::WorldGuardrailPredictionContext::from_scores(
            archon_world_model::RuntimeTaskClass::CodingChange,
            archon_world_model::WorldGuardrailMode::Guarded,
            guardrail_scores_for_prediction(
                archon_world_model::RuntimeTaskClass::CodingChange,
                None,
            ),
            &policy,
        );
        let default_decision = archon_world_model::guardrail::decide_guardrail(
            &action,
            None,
            default_context,
            &policy,
        );
        let mut prediction = archon_world_model::WorldPrediction::new("model-1", "low risk");
        prediction.guardrail_scores = Some(archon_world_model::GuardrailRiskScores {
            predicted_failure: Some(0.05),
            predicted_verification_needed: Some(0.05),
            predicted_user_correction: Some(0.05),
            predicted_plan_drift: Some(0.05),
            ..archon_world_model::GuardrailRiskScores::default()
        });
        let learned_context = archon_world_model::WorldGuardrailPredictionContext::from_scores(
            archon_world_model::RuntimeTaskClass::CodingChange,
            archon_world_model::WorldGuardrailMode::Guarded,
            guardrail_scores_for_prediction(
                archon_world_model::RuntimeTaskClass::CodingChange,
                Some(&prediction),
            ),
            &policy,
        );
        let learned_decision = archon_world_model::guardrail::decide_guardrail(
            &action,
            Some(&prediction),
            learned_context,
            &policy,
        );

        assert!(!default_decision.allowed_to_finalize);
        assert!(learned_decision.allowed_to_finalize);
        assert_ne!(
            default_decision.allowed_to_finalize,
            learned_decision.allowed_to_finalize
        );
    }

    #[test]
    fn learned_high_scores_change_pipeline_decision_from_allow_to_block() {
        let policy = archon_world_model::WorldGuardrailPolicyConfig::default();
        let action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::PipelineStep,
            archon_world_model::GuardedActionKind::PipelineStep,
            "run pipeline",
            "pipeline batch",
        );
        let default_context = archon_world_model::WorldGuardrailPredictionContext::from_scores(
            archon_world_model::RuntimeTaskClass::PipelineExecution,
            archon_world_model::WorldGuardrailMode::Guarded,
            guardrail_scores_for_prediction(
                archon_world_model::RuntimeTaskClass::PipelineExecution,
                None,
            ),
            &policy,
        );
        let default_decision = archon_world_model::guardrail::decide_guardrail(
            &action,
            None,
            default_context,
            &policy,
        );
        let mut prediction = archon_world_model::WorldPrediction::new("model-1", "high risk");
        prediction.guardrail_scores = Some(archon_world_model::GuardrailRiskScores {
            predicted_failure: Some(0.91),
            predicted_verification_needed: Some(0.91),
            ..archon_world_model::GuardrailRiskScores::default()
        });
        let learned_context = archon_world_model::WorldGuardrailPredictionContext::from_scores(
            archon_world_model::RuntimeTaskClass::PipelineExecution,
            archon_world_model::WorldGuardrailMode::Guarded,
            guardrail_scores_for_prediction(
                archon_world_model::RuntimeTaskClass::PipelineExecution,
                Some(&prediction),
            ),
            &policy,
        );
        let learned_decision = archon_world_model::guardrail::decide_guardrail(
            &action,
            Some(&prediction),
            learned_context,
            &policy,
        );

        assert!(default_decision.allowed_to_finalize);
        assert!(!learned_decision.allowed_to_finalize);
        assert_ne!(
            default_decision.allowed_to_finalize,
            learned_decision.allowed_to_finalize
        );
    }

