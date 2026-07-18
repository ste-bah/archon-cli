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
fn first_tool_reclassifies_active_guardrail_once_and_preserves_identity() {
    let temp = tempfile::tempdir().unwrap();
    let config = guarded_test_config();
    let initial = runtime_guardrail_record("tool-class-session", "tool-class-action");
    remember_active_guardrail(&initial);

    reclassify_active_guardrail_at_root(
        &config,
        temp.path(),
        "tool-class-session",
        "tool-class-action",
        "Edit",
        "tool-use-1",
        &serde_json::json!({"file_path": "src/lib.rs"}),
    );
    reclassify_active_guardrail_at_root(
        &config,
        temp.path(),
        "tool-class-session",
        "tool-class-action",
        "WebSearch",
        "tool-use-2",
        &serde_json::json!({"query": "ignored second tool"}),
    );

    let current =
        active_guardrail_for_session("tool-class-session").expect("reclassified guardrail");
    assert_eq!(current.action.action_id, initial.action.action_id);
    assert_eq!(current.action.session_id, initial.action.session_id);
    assert_eq!(
        current.task_class,
        archon_world_model::RuntimeTaskClass::CodingChange
    );
    assert!(current.classified_from_tool);
    assert_eq!(
        current
            .decision
            .prediction_context
            .as_ref()
            .map(|context| context.task_class),
        Some(archon_world_model::RuntimeTaskClass::CodingChange)
    );
    clear_active_guardrail("tool-class-session", "tool-class-action");
}

#[test]
fn first_read_locks_general_answer_classification() {
    let temp = tempfile::tempdir().unwrap();
    let config = guarded_test_config();
    let initial = runtime_guardrail_record("read-lock-session", "read-lock-action");
    remember_active_guardrail(&initial);

    reclassify_active_guardrail_at_root(
        &config,
        temp.path(),
        "read-lock-session",
        "read-lock-action",
        "Read",
        "tool-use-read",
        &serde_json::json!({"file_path": "src/lib.rs"}),
    );
    reclassify_active_guardrail_at_root(
        &config,
        temp.path(),
        "read-lock-session",
        "read-lock-action",
        "Edit",
        "tool-use-edit",
        &serde_json::json!({"file_path": "src/lib.rs"}),
    );

    let current = active_guardrail_for_session("read-lock-session").unwrap();
    assert!(current.classified_from_tool);
    assert_eq!(
        current.task_class,
        archon_world_model::RuntimeTaskClass::GeneralAnswer
    );
    clear_active_guardrail("read-lock-session", "read-lock-action");
}

#[test]
fn completion_uses_current_reclassified_record_instead_of_stale_clone() {
    let initial = runtime_guardrail_record("completion-session", "completion-action");
    remember_active_guardrail(&initial);
    let mut current = initial.clone();
    current.task_class = archon_world_model::RuntimeTaskClass::CodingChange;
    current.classified_from_tool = true;
    remember_active_guardrail(&current);

    let selected = current_record_for_completion(&initial);

    assert_eq!(
        selected.task_class,
        archon_world_model::RuntimeTaskClass::CodingChange
    );
    clear_active_guardrail("completion-session", "completion-action");
}

fn guarded_test_config() -> archon_core::config::ArchonConfig {
    let mut config = archon_core::config::ArchonConfig::default();
    config.learning.world_model.guardrails.interactive_mode = "guarded".into();
    config
}

fn runtime_guardrail_record(session_id: &str, action_id: &str) -> RuntimeGuardrailRecord {
    let mut action = archon_world_model::WorldGuardedAction::new(
        session_id,
        archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
        archon_world_model::GuardedActionKind::UserRequest,
        "prompt wording is provisional",
        "prompt wording is provisional",
    );
    action.action_id = action_id.into();
    let mut prediction = archon_world_model::WorldPrediction::new("model-1", "next state");
    prediction.guardrail_scores = None;
    let advisory = archon_world_model::integration::WorldAdvisorSurfaceRecord {
        surface: action.surface,
        prediction: Some(prediction),
        unavailable: None,
        session_id: Some(session_id.into()),
        action_ref: Some(action_id.into()),
        action_summary: Some(action.action_summary.clone()),
        continue_foreground_flow: true,
        created_at: chrono::Utc::now(),
    };
    RuntimeGuardrailRecord {
        decision: archon_world_model::WorldGuardrailDecision::unavailable(&action),
        action,
        advisory,
        task_class: archon_world_model::RuntimeTaskClass::GeneralAnswer,
        classified_from_tool: false,
    }
}

#[test]
fn queued_guardrail_does_not_steal_inflight_first_tool() {
    let temp = tempfile::tempdir().unwrap();
    let config = guarded_test_config();
    let inflight = runtime_guardrail_record("queued-session", "inflight-action");
    let queued = runtime_guardrail_record("queued-session", "queued-action");
    remember_active_guardrail(&inflight);
    remember_active_guardrail(&queued);

    reclassify_active_guardrail_at_root(
        &config,
        temp.path(),
        "queued-session",
        "inflight-action",
        "Edit",
        "inflight-tool",
        &serde_json::json!({"file_path": "src/lib.rs"}),
    );

    let current = active_guardrail_for_session("queued-session").unwrap();
    assert_eq!(current.action.action_id, "inflight-action");
    assert!(current.classified_from_tool);
    assert_eq!(
        current.task_class,
        archon_world_model::RuntimeTaskClass::CodingChange
    );
    let queued_current = current_record_for_completion(&queued);
    assert_eq!(queued_current.action.action_id, "queued-action");
    assert!(!queued_current.classified_from_tool);
    clear_active_guardrail("queued-session", "inflight-action");
    assert_eq!(
        active_guardrail_for_session("queued-session")
            .expect("queued guardrail becomes active")
            .action
            .action_id,
        "queued-action"
    );
    clear_active_guardrail("queued-session", "queued-action");
}

#[test]
fn classified_blocked_guardrail_does_not_capture_queued_turn_tool() {
    let temp = tempfile::tempdir().unwrap();
    let config = guarded_test_config();
    let mut blocked = runtime_guardrail_record("blocked-queue-session", "blocked-action");
    blocked.classified_from_tool = true;
    blocked.decision.mode = archon_world_model::WorldGuardrailMode::Guarded;
    blocked.decision.allowed_to_finalize = false;
    blocked.decision.required_actions = vec![archon_world_model::GuardrailRequiredAction::RunTests];
    let queued = runtime_guardrail_record("blocked-queue-session", "queued-action");
    remember_active_guardrail(&blocked);
    remember_active_guardrail(&queued);

    reclassify_active_guardrail_at_root(
        &config,
        temp.path(),
        "blocked-queue-session",
        "queued-action",
        "WebSearch",
        "queued-tool",
        &serde_json::json!({"query": "current turn"}),
    );

    let queued_current = current_record_for_completion(&queued);
    assert!(queued_current.classified_from_tool);
    assert_eq!(
        queued_current.task_class,
        archon_world_model::RuntimeTaskClass::ResearchAnswer
    );
    assert_eq!(
        active_guardrail_for_session("blocked-queue-session")
            .expect("executing queued guardrail")
            .action
            .action_id,
        "queued-action"
    );
    let blocked_current = active_guardrail_for_action("blocked-queue-session", "blocked-action")
        .expect("blocked guardrail remains tracked");
    assert!(blocked_current.classified_from_tool);
    clear_active_guardrail("blocked-queue-session", "blocked-action");
    clear_active_guardrail("blocked-queue-session", "queued-action");
}

#[test]
fn activating_queued_turn_promotes_its_guardrail_before_tool_use() {
    let session_id = "turn-activation-session";
    let blocked = runtime_guardrail_record(session_id, "blocked-action");
    let queued = runtime_guardrail_record(session_id, "queued-action");
    remember_active_guardrail(&blocked);
    remember_active_guardrail(&queued);

    activate_guardrail_for_action(session_id, "queued-action");

    assert_eq!(
        active_guardrail_for_session(session_id)
            .expect("queued turn is active")
            .action
            .action_id,
        "queued-action"
    );
    clear_active_guardrail(session_id, "blocked-action");
    clear_active_guardrail(session_id, "queued-action");
}

#[test]
fn failed_initial_persistence_does_not_activate_guardrail() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ledgers"), "not a directory").unwrap();
    let record = runtime_guardrail_record("initial-fail-session", "initial-fail-action");

    assert!(persist_and_remember_guardrail(temp.path(), &record).is_err());
    assert!(active_guardrail_for_session("initial-fail-session").is_none());
}

#[test]
fn failed_revision_persistence_keeps_prior_runtime_classification() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ledgers"), "not a directory").unwrap();
    let config = guarded_test_config();
    let initial = runtime_guardrail_record("persist-fail-session", "persist-fail-action");
    remember_active_guardrail(&initial);

    reclassify_active_guardrail_at_root(
        &config,
        temp.path(),
        "persist-fail-session",
        "persist-fail-action",
        "Edit",
        "persist-fail-tool",
        &serde_json::json!({"file_path": "src/lib.rs"}),
    );

    let current = current_record_for_completion(&initial);
    assert!(!current.classified_from_tool);
    assert_eq!(current.task_class, archon_world_model::RuntimeTaskClass::GeneralAnswer);
    assert_eq!(current.decision, initial.decision);
    clear_active_guardrail("persist-fail-session", "persist-fail-action");
}

#[test]
fn reclassification_persists_revised_verification_plan() {
    let temp = tempfile::tempdir().unwrap();
    let config = guarded_test_config();
    let initial = runtime_guardrail_record("persist-plan-session", "persist-plan-action");
    archon_world_model::guardrail::append_guarded_action(temp.path(), &initial.action).unwrap();
    remember_active_guardrail(&initial);

    reclassify_active_guardrail_at_root(
        &config,
        temp.path(),
        "persist-plan-session",
        "persist-plan-action",
        "Edit",
        "persist-plan-tool",
        &serde_json::json!({"file_path": "src/lib.rs"}),
    );

    let actions = archon_world_model::guardrail::load_guarded_actions(temp.path()).unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].action_id, initial.action.action_id);
    assert!(
        actions[0]
            .verification_plan
            .iter()
            .any(|requirement| requirement.kind == archon_world_model::VerificationKind::UnitTests)
    );
    clear_active_guardrail("persist-plan-session", "persist-plan-action");
}

#[test]
fn forced_repair_prompt_uses_current_reclassified_record() {
    let stale = runtime_guardrail_record("repair-session", "repair-action");
    remember_active_guardrail(&stale);
    let mut current = stale.clone();
    current.task_class = archon_world_model::RuntimeTaskClass::CodingChange;
    current.classified_from_tool = true;
    current.decision.mode = archon_world_model::WorldGuardrailMode::Guarded;
    current.decision.allowed_to_finalize = false;
    current.decision.required_actions = vec![archon_world_model::GuardrailRequiredAction::RunTests];
    remember_active_guardrail(&current);

    let prompt = forced_repair_prompt(&stale).expect("current record should force repair");

    assert!(prompt.contains("RunTests"));
    clear_active_guardrail("repair-session", "repair-action");
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
    let scores =
        guardrail_scores_for_prediction(archon_world_model::RuntimeTaskClass::CodingChange, None);

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
        guardrail_scores_for_prediction(archon_world_model::RuntimeTaskClass::CodingChange, None),
        &policy,
    );
    let default_decision =
        archon_world_model::guardrail::decide_guardrail(&action, None, default_context, &policy);
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
    let default_decision =
        archon_world_model::guardrail::decide_guardrail(&action, None, default_context, &policy);
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
