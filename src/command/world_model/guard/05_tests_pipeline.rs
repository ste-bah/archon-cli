    #[test]
    fn pipeline_agent_name_and_quality_do_not_satisfy_run_tests() {
        let parent = pipeline_parent_record(
            archon_world_model::GuardrailRequiredAction::RunTests,
            archon_world_model::VerificationKind::UnitTests,
        );
        let agent = pipeline_agent("quality-test-writer", "Quality Test Writer", false, 0.70);
        let result = pipeline_agent_result(0.99);

        let verification = pipeline_agent_verification(
            &parent,
            "parent:step:1:quality-test-writer",
            &agent,
            &result,
        );
        let verifications = verification.into_iter().collect::<Vec<_>>();

        assert!(
            verifications.iter().all(|verification| verification.status
                != archon_world_model::VerificationStatus::Passed)
        );
        assert!(!archon_world_model::guardrail::finalization_allowed(
            &parent.decision,
            &verifications
        ));
    }

    #[test]
    fn pipeline_real_failed_verification_ignores_high_quality() {
        let parent = pipeline_parent_record(
            archon_world_model::GuardrailRequiredAction::RunTests,
            archon_world_model::VerificationKind::UnitTests,
        );
        let agent = pipeline_agent("integration-verifier", "Integration Verifier", true, 0.70);
        let result = pipeline_agent_result_with_tools(
            0.99,
            vec![pipeline_tool(
                "Bash",
                "cargo test",
                serde_json::json!({"exit_code": 1}),
            )],
        );

        let verification = pipeline_agent_verification(
            &parent,
            "parent:step:2:integration-verifier",
            &agent,
            &result,
        )
        .expect("real test command should produce verification evidence");

        assert_eq!(
            verification.kind,
            archon_world_model::VerificationKind::UnitTests
        );
        assert_eq!(
            verification.status,
            archon_world_model::VerificationStatus::Failed
        );
        assert_eq!(verification.exit_code, Some(1));
        assert!(!archon_world_model::guardrail::finalization_allowed(
            &parent.decision,
            &[verification]
        ));
    }

    #[test]
    fn pipeline_real_passed_verification_allows_requirement() {
        let parent = pipeline_parent_record(
            archon_world_model::GuardrailRequiredAction::RunBuild,
            archon_world_model::VerificationKind::Build,
        );
        let agent = pipeline_agent("build-verifier", "Build Verifier", true, 0.70);
        let result = pipeline_agent_result_with_tools(
            0.20,
            vec![pipeline_tool(
                "Bash",
                "cargo check",
                serde_json::json!({"exit_code": 0, "summary": "cargo check passed"}),
            )],
        );

        let verification =
            pipeline_agent_verification(&parent, "parent:step:3:build-verifier", &agent, &result)
                .expect("real build command should produce verification evidence");

        assert_eq!(
            verification.kind,
            archon_world_model::VerificationKind::Build
        );
        assert_eq!(
            verification.status,
            archon_world_model::VerificationStatus::Passed
        );
        assert_eq!(verification.exit_code, Some(0));
        assert!(archon_world_model::guardrail::finalization_allowed(
            &parent.decision,
            &[verification]
        ));
    }

    #[test]
    fn pipeline_verification_command_without_execution_signal_is_inconclusive() {
        let parent = pipeline_parent_record(
            archon_world_model::GuardrailRequiredAction::RunTests,
            archon_world_model::VerificationKind::UnitTests,
        );
        let agent = pipeline_agent("test-runner", "Test Runner", true, 0.70);
        let result = pipeline_agent_result_with_tools(
            0.99,
            vec![pipeline_tool("Bash", "cargo test", serde_json::Value::Null)],
        );

        let verification =
            pipeline_agent_verification(&parent, "parent:step:4:test-runner", &agent, &result)
                .expect("verification command should record an inconclusive outcome");

        assert_eq!(
            verification.status,
            archon_world_model::VerificationStatus::Inconclusive
        );
        assert!(verification.summary.contains("no_execution_signal"));
        assert!(!archon_world_model::guardrail::finalization_allowed(
            &parent.decision,
            &[verification]
        ));
    }

    fn pipeline_parent_record(
        required: archon_world_model::GuardrailRequiredAction,
        kind: archon_world_model::VerificationKind,
    ) -> RuntimeGuardrailRecord {
        let mut action = archon_world_model::WorldGuardedAction::new(
            "pipeline-session",
            archon_world_model::integration::WorldAdvisorSurface::PipelineStep,
            archon_world_model::GuardedActionKind::PipelineStep,
            "coding pipeline: implement feature",
            "coding pipeline: implement feature",
        );
        action.action_id = "parent".into();
        action.verification_plan = vec![archon_world_model::VerificationRequirement {
            requirement_id: "world-guard-req-parent-run-tests".into(),
            kind,
            command_hint: None,
            applies_to: vec![action.action_id.clone()],
            required_for_final: true,
        }];
        let mut decision = archon_world_model::WorldGuardrailDecision::default();
        decision.action_id = action.action_id.clone();
        decision.required_actions = vec![required];
        decision.mode = archon_world_model::WorldGuardrailMode::Guarded;
        decision.allowed_to_finalize = false;
        RuntimeGuardrailRecord {
            action,
            advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord::unavailable(
                archon_world_model::integration::WorldAdvisorSurface::PipelineStep,
                archon_world_model::WorldAdvisorUnavailableReason::ColdStart,
            ),
            decision,
            task_class: archon_world_model::RuntimeTaskClass::PipelineExecution,
        }
    }

    fn pipeline_agent(
        key: &str,
        display_name: &str,
        critical: bool,
        quality_threshold: f64,
    ) -> archon_pipeline::runner::AgentInfo {
        archon_pipeline::runner::AgentInfo {
            key: key.into(),
            display_name: display_name.into(),
            model: "test-model".into(),
            phase: 5,
            critical,
            quality_threshold,
            tool_access_level: archon_pipeline::runner::ToolAccessLevel::ReadOnly,
        }
    }

    fn pipeline_agent_result(overall: f64) -> archon_pipeline::runner::AgentResult {
        pipeline_agent_result_with_tools(overall, Vec::new())
    }

    fn pipeline_agent_result_with_tools(
        overall: f64,
        tool_use_log: Vec<archon_pipeline::runner::ToolUseEntry>,
    ) -> archon_pipeline::runner::AgentResult {
        archon_pipeline::runner::AgentResult {
            output: "checked".into(),
            tool_use_log,
            tokens_in: 10,
            tokens_out: 20,
            cost_usd: 0.0,
            duration: std::time::Duration::from_millis(25),
            quality: Some(archon_pipeline::runner::QualityScore {
                overall,
                dimensions: std::collections::HashMap::new(),
            }),
        }
    }

    fn pipeline_tool(
        tool_name: &str,
        command: &str,
        output: serde_json::Value,
    ) -> archon_pipeline::runner::ToolUseEntry {
        archon_pipeline::runner::ToolUseEntry {
            tool_name: tool_name.into(),
            input: serde_json::json!({ "command": command }),
            output,
        }
    }

