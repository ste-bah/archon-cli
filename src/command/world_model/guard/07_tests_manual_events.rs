    #[test]
    fn approve_records_manual_override_and_skipped_requirements() {
        let temp = tempfile::tempdir().unwrap();
        let mut action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "build app",
            "build app",
        );
        action.action_id = "manual-approve-action".into();
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        decision.surface = action.surface;
        decision.mode = archon_world_model::WorldGuardrailMode::Guarded;
        decision.allowed_to_finalize = false;
        decision.required_actions = vec![
            archon_world_model::GuardrailRequiredAction::RunTests,
            archon_world_model::GuardrailRequiredAction::RunBuild,
        ];
        action.verification_plan = verification_plan_for_decision(&action.action_id, &decision);
        archon_world_model::guardrail::append_guarded_action(temp.path(), &action).unwrap();
        archon_world_model::guardrail::append_guardrail_decision(temp.path(), &decision).unwrap();

        let rendered =
            render_guard_approve(temp.path(), &action.action_id, "operator accepts risk").unwrap();
        let verifications =
            archon_world_model::guardrail::load_verification_outcomes(temp.path()).unwrap();
        let outcomes = archon_world_model::guardrail::load_guardrail_outcomes(temp.path()).unwrap();

        assert!(rendered.contains("user_approved_despite_risk"));
        assert_eq!(verifications.len(), 2);
        assert!(
            verifications.iter().all(|verification| verification.status
                == archon_world_model::VerificationStatus::Skipped)
        );
        assert!(archon_world_model::guardrail::finalization_allowed(
            &decision,
            &verifications
        ));
        assert_eq!(
            outcomes[0].final_status,
            archon_world_model::GuardrailFinalStatus::UserApprovedDespiteRisk
        );
        assert_eq!(outcomes[0].verification_outcomes.len(), 2);
    }

    #[test]
    fn skip_verification_records_skipped_outcome() {
        let temp = tempfile::tempdir().unwrap();
        let mut action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "build app",
            "build app",
        );
        action.action_id = "manual-skip-action".into();
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        decision.required_actions = vec![archon_world_model::GuardrailRequiredAction::RunTests];
        action.verification_plan = verification_plan_for_decision(&action.action_id, &decision);
        let requirement_id = action.verification_plan[0].requirement_id.clone();
        archon_world_model::guardrail::append_guarded_action(temp.path(), &action).unwrap();

        let rendered =
            render_guard_skip_verification(temp.path(), &requirement_id, "test host unavailable")
                .unwrap();
        let verifications =
            archon_world_model::guardrail::load_verification_outcomes(temp.path()).unwrap();

        assert!(rendered.contains("Status:      skipped"));
        assert_eq!(verifications.len(), 1);
        assert_eq!(verifications[0].requirement_id, requirement_id);
        assert_eq!(
            verifications[0].status,
            archon_world_model::VerificationStatus::Skipped
        );
        assert!(verifications[0].summary.contains("test host unavailable"));
    }

    #[test]
    fn provider_incident_attaches_to_active_guardrail_observations() {
        let session_id = format!("s-{}", uuid::Uuid::new_v4());
        let mut action = archon_world_model::WorldGuardedAction::new(
            &session_id,
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "build app",
            "build app",
        );
        action.action_id = format!("a-{}", uuid::Uuid::new_v4());
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        let record = RuntimeGuardrailRecord {
            action,
            advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord::unavailable(
                archon_world_model::integration::WorldAdvisorSurface::CodingTask,
                archon_world_model::WorldAdvisorUnavailableReason::ColdStart,
            ),
            decision,
            task_class: archon_world_model::RuntimeTaskClass::CodingChange,
        };
        remember_active_guardrail(&record);

        let attached = record_guardrail_provider_incident_for_session(
            &archon_core::config::ArchonConfig::default(),
            &session_id,
            "provider-event-1",
            "rate_limited",
        );
        let observations = observations_for(&record.action.action_id);

        assert!(attached);
        assert!(observations.provider_incident_observed);
        assert_eq!(observations.retry_count, 1);
        assert!(
            observations
                .evidence_refs
                .contains(&"provider_event:provider-event-1".to_string())
        );
        clear_active_guardrail(&session_id, &record.action.action_id);
    }

    #[test]
    fn reasoning_quality_event_attaches_to_active_guardrail_observations() {
        let session_id = format!("s-{}", uuid::Uuid::new_v4());
        let mut action = archon_world_model::WorldGuardedAction::new(
            &session_id,
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "build app",
            "build app",
        );
        action.action_id = format!("a-{}", uuid::Uuid::new_v4());
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        let record = RuntimeGuardrailRecord {
            action,
            advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord::unavailable(
                archon_world_model::integration::WorldAdvisorSurface::CodingTask,
                archon_world_model::WorldAdvisorUnavailableReason::ColdStart,
            ),
            decision,
            task_class: archon_world_model::RuntimeTaskClass::CodingChange,
        };
        remember_active_guardrail(&record);
        let event = archon_reasoning_quality::ReasoningQualityEvent {
            event_id: "rqevt-1".into(),
            session_id: session_id.clone(),
            event_kind: archon_reasoning_quality::ReasoningEventKind::ClaimCorrectedByUser,
            ..archon_reasoning_quality::ReasoningQualityEvent::default()
        };

        let attached = record_guardrail_reasoning_quality_event(&event);
        let observations = observations_for(&record.action.action_id);

        assert!(attached);
        assert!(observations.user_correction_observed);
        assert!(observations.reasoning_failure_observed);
        assert!(
            observations
                .evidence_refs
                .contains(&"reasoning_quality:rqevt-1".to_string())
        );
        clear_active_guardrail(&session_id, &record.action.action_id);
    }
