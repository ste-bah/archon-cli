    #[test]
    fn status_reports_guardrail_counts() {
        let temp = tempfile::tempdir().unwrap();
        let action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
            archon_world_model::GuardedActionKind::UserRequest,
            "goal",
            "summary",
        );
        archon_world_model::guardrail::append_guarded_action(temp.path(), &action).unwrap();

        let rendered =
            render_guard_status(&archon_core::config::ArchonConfig::default(), temp.path());

        assert!(rendered.contains("World Model Guardrails"));
        assert!(rendered.contains("Interactive mode:"));
        assert!(rendered.contains("Actions:                  1"));
    }

    #[test]
    fn status_reports_override_surprise_and_unavailable_breakdown() {
        let temp = tempfile::tempdir().unwrap();
        let mut action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
            archon_world_model::GuardedActionKind::UserRequest,
            "goal",
            "summary",
        );
        action.action_id = "status-action".into();
        let decision = archon_world_model::WorldGuardrailDecision::unavailable(&action);
        let mut outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
            &decision,
            archon_world_model::RuntimeTaskClass::CodingChange,
            archon_world_model::GuardrailFinalStatus::UserApprovedDespiteRisk,
            "approved after manual check",
        );
        outcome.latent_surprise = Some(0.55);
        archon_world_model::guardrail::append_guarded_action(temp.path(), &action).unwrap();
        archon_world_model::guardrail::append_guardrail_decision(temp.path(), &decision).unwrap();
        archon_world_model::guardrail::append_guardrail_outcome(temp.path(), &outcome).unwrap();

        let rendered =
            render_guard_status(&archon_core::config::ArchonConfig::default(), temp.path());

        assert!(rendered.contains("Unavailable reasons:      AdvisorUnavailable=1"));
        assert!(rendered.contains("User overrides:           1"));
        assert!(rendered.contains("High-surprise outcomes:   1"));
        assert!(rendered.contains("Latest actions:           status-action:InteractiveSession"));
    }

    #[test]
    fn guard_list_filters_by_derived_status() {
        let temp = tempfile::tempdir().unwrap();
        let mut blocked = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "blocked",
            "blocked",
        );
        blocked.action_id = "blocked-action".into();
        let blocked_decision = archon_world_model::WorldGuardrailDecision::unavailable(&blocked);
        let blocked_outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
            &blocked_decision,
            archon_world_model::RuntimeTaskClass::CodingChange,
            archon_world_model::GuardrailFinalStatus::BlockedMissingVerification,
            "missing tests",
        );

        let mut complete = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "complete",
            "complete",
        );
        complete.action_id = "complete-action".into();
        let complete_decision = archon_world_model::WorldGuardrailDecision::unavailable(&complete);
        let complete_outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
            &complete_decision,
            archon_world_model::RuntimeTaskClass::CodingChange,
            archon_world_model::GuardrailFinalStatus::CompletedVerified,
            "done",
        );

        let mut open = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "open",
            "open",
        );
        open.action_id = "open-action".into();
        let mut open_decision = archon_world_model::WorldGuardrailDecision::default();
        open_decision.action_id = open.action_id.clone();
        open_decision.mode = archon_world_model::WorldGuardrailMode::Guarded;
        open_decision.allowed_to_finalize = false;
        open_decision.required_actions =
            vec![archon_world_model::GuardrailRequiredAction::RunTests];

        for action in [&blocked, &complete, &open] {
            archon_world_model::guardrail::append_guarded_action(temp.path(), action).unwrap();
        }
        archon_world_model::guardrail::append_guardrail_decision(temp.path(), &blocked_decision)
            .unwrap();
        archon_world_model::guardrail::append_guardrail_decision(temp.path(), &complete_decision)
            .unwrap();
        archon_world_model::guardrail::append_guardrail_decision(temp.path(), &open_decision)
            .unwrap();
        archon_world_model::guardrail::append_guardrail_outcome(temp.path(), &blocked_outcome)
            .unwrap();
        archon_world_model::guardrail::append_guardrail_outcome(temp.path(), &complete_outcome)
            .unwrap();

        let blocked_list = render_guard_list(temp.path(), None, None, Some("blocked")).unwrap();
        let open_list = render_guard_list(temp.path(), None, None, Some("open")).unwrap();
        let complete_list = render_guard_list(temp.path(), None, None, Some("complete")).unwrap();

        assert!(blocked_list.contains("blocked-action [blocked]"));
        assert!(!blocked_list.contains("open-action"));
        assert!(open_list.contains("open-action [open]"));
        assert!(!open_list.contains("complete-action"));
        assert!(complete_list.contains("complete-action [complete]"));
        assert!(!complete_list.contains("blocked-action"));
    }

    #[test]
    fn blocked_guardrail_record_produces_forced_repair_prompt() {
        let action = archon_world_model::WorldGuardedAction::new(
            "s1",
            archon_world_model::integration::WorldAdvisorSurface::CodingTask,
            archon_world_model::GuardedActionKind::UserRequest,
            "build app",
            "build app",
        );
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        decision.mode = archon_world_model::WorldGuardrailMode::Guarded;
        decision.allowed_to_finalize = false;
        decision.required_actions = vec![archon_world_model::GuardrailRequiredAction::RunTests];
        let record = RuntimeGuardrailRecord {
            action,
            advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord::unavailable(
                archon_world_model::integration::WorldAdvisorSurface::CodingTask,
                archon_world_model::WorldAdvisorUnavailableReason::ColdStart,
            ),
            decision,
            task_class: archon_world_model::RuntimeTaskClass::CodingChange,
        };

        let prompt = forced_repair_prompt(&record).expect("blocked record should force repair");

        assert!(prompt.contains("must not be marked complete"));
        assert!(prompt.contains("RunTests"));
    }

    #[test]
    fn classify_tool_command_detects_verification_commands() {
        let (kind, verification) = classify_tool_command("cargo test -p archon-core");
        assert_eq!(kind, archon_world_model::GuardedActionKind::TestCommand);
        assert_eq!(
            verification,
            Some(archon_world_model::VerificationKind::UnitTests)
        );

        let (kind, verification) = classify_tool_command("cargo check --bin archon");
        assert_eq!(kind, archon_world_model::GuardedActionKind::BuildCommand);
        assert_eq!(
            verification,
            Some(archon_world_model::VerificationKind::Build)
        );

        let (kind, verification) = classify_tool_command("python scripts/one_off.py");
        assert_eq!(kind, archon_world_model::GuardedActionKind::ShellCommand);
        assert_eq!(verification, None);
    }

    #[test]
    fn verification_plan_uses_stable_requirement_ids() {
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.required_actions = vec![
            archon_world_model::GuardrailRequiredAction::RunTests,
            archon_world_model::GuardrailRequiredAction::RunBuild,
        ];

        let plan = verification_plan_for_decision("action-123", &decision);

        assert_eq!(plan.len(), 2);
        assert!(
            plan.iter()
                .any(|req| req.requirement_id == "world-guard-req-action-123-run-tests")
        );
        assert!(
            plan.iter()
                .any(|req| req.requirement_id == "world-guard-req-action-123-run-build")
        );
    }

