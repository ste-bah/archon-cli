#[test]
fn turn_requirements_render_exact_action() {
    let first = blocking_turn_record("requirements-session", "requirements-first");
    let second = blocking_turn_record("requirements-session", "requirements-second");
    remember_active_guardrail(&first);
    remember_active_guardrail(&second);

    let rendered = turn_requirements_for_action("requirements-session", "requirements-second")
        .expect("second action requirements");

    assert!(rendered.contains("requirements-second"));
    assert!(rendered.contains("RunTests"));
    assert!(!rendered.contains("requirements-first"));
    clear_active_guardrail("requirements-session", "requirements-first");
    clear_active_guardrail("requirements-session", "requirements-second");
}

#[test]
fn finalization_verdict_blocks_missing_and_failed_verification() {
    let temp = tempfile::tempdir().unwrap();
    let record = blocking_turn_record("finalization-session", "finalization-action");
    remember_active_guardrail(&record);

    let missing = turn_finalization_verdict_at_root(
        temp.path(),
        "finalization-session",
        "finalization-action",
    );
    assert!(matches!(
        missing,
        archon_core::agent::TurnFinalizationVerdict::Blocked { .. }
    ));

    let mut failed = verification_for(&record, archon_world_model::VerificationStatus::Failed);
    failed.created_at = chrono::Utc::now();
    archon_world_model::guardrail::append_verification_outcome(temp.path(), &failed).unwrap();
    let failed_verdict = turn_finalization_verdict_at_root(
        temp.path(),
        "finalization-session",
        "finalization-action",
    );
    assert!(matches!(
        failed_verdict,
        archon_core::agent::TurnFinalizationVerdict::Blocked { .. }
    ));
    clear_active_guardrail("finalization-session", "finalization-action");
}

#[test]
fn finalization_verdict_uses_latest_evidence_and_manual_override() {
    let temp = tempfile::tempdir().unwrap();
    let record = blocking_turn_record("latest-session", "latest-action");
    remember_active_guardrail(&record);

    let mut failed = verification_for(&record, archon_world_model::VerificationStatus::Failed);
    failed.created_at = chrono::Utc::now() - chrono::Duration::seconds(1);
    archon_world_model::guardrail::append_verification_outcome(temp.path(), &failed).unwrap();
    let passed = verification_for(&record, archon_world_model::VerificationStatus::Passed);
    archon_world_model::guardrail::append_verification_outcome(temp.path(), &passed).unwrap();
    assert_eq!(
        turn_finalization_verdict_at_root(temp.path(), "latest-session", "latest-action"),
        archon_core::agent::TurnFinalizationVerdict::Allowed
    );

    let override_record = blocking_turn_record("override-session", "override-action");
    remember_active_guardrail(&override_record);
    let mut manual = verification_for(
        &override_record,
        archon_world_model::VerificationStatus::Skipped,
    );
    manual.evidence_refs = vec!["manual_override:approve".into()];
    archon_world_model::guardrail::append_verification_outcome(temp.path(), &manual).unwrap();
    assert_eq!(
        turn_finalization_verdict_at_root(temp.path(), "override-session", "override-action"),
        archon_core::agent::TurnFinalizationVerdict::Allowed
    );
    clear_active_guardrail("latest-session", "latest-action");
    clear_active_guardrail("override-session", "override-action");
}

#[test]
fn finalization_ordering_matches_durable_status_when_append_order_differs() {
    let temp = tempfile::tempdir().unwrap();
    let record = blocking_turn_record("ordering-session", "ordering-action");
    remember_active_guardrail(&record);

    let mut passed = verification_for(&record, archon_world_model::VerificationStatus::Passed);
    passed.created_at = chrono::Utc::now();
    passed.idempotency_key = "verification:ordering:passed-newer".into();
    let mut failed = verification_for(&record, archon_world_model::VerificationStatus::Failed);
    failed.created_at = passed.created_at - chrono::Duration::seconds(1);
    failed.idempotency_key = "verification:ordering:failed-older".into();
    archon_world_model::guardrail::append_verification_outcome(temp.path(), &passed).unwrap();
    archon_world_model::guardrail::append_verification_outcome(temp.path(), &failed).unwrap();

    let outcomes = archon_world_model::guardrail::load_verification_outcomes(temp.path()).unwrap();
    assert!(archon_world_model::guardrail::finalization_allowed(
        &record.decision,
        &outcomes,
    ));
    assert_eq!(
        turn_finalization_verdict_at_root(temp.path(), "ordering-session", "ordering-action"),
        archon_core::agent::TurnFinalizationVerdict::Allowed
    );
    clear_active_guardrail("ordering-session", "ordering-action");
}

#[test]
fn bare_skipped_verification_does_not_allow_finalization() {
    let temp = tempfile::tempdir().unwrap();
    let record = blocking_turn_record("skip-session", "skip-action");
    remember_active_guardrail(&record);
    let skipped = verification_for(&record, archon_world_model::VerificationStatus::Skipped);
    archon_world_model::guardrail::append_verification_outcome(temp.path(), &skipped).unwrap();

    assert!(matches!(
        turn_finalization_verdict_at_root(temp.path(), "skip-session", "skip-action"),
        archon_core::agent::TurnFinalizationVerdict::Blocked { .. }
    ));
    clear_active_guardrail("skip-session", "skip-action");
}


#[test]
fn finalization_uses_the_runtime_session_database_not_the_default_store() {
    let world_model = tempfile::tempdir().unwrap();
    let session_database = world_model.path().join("runtime-session.db");
    let session_id = "runtime-session";
    let store = archon_session::storage::SessionStore::open(&session_database).unwrap();
    let plans = archon_session::plan::PlanStore::new(store.db()).unwrap();
    let mut plan = archon_session::plan::PlanDocument::new("runtime-plan", "Runtime plan");
    plan.session_id = Some(session_id.into());
    plan.status = archon_session::plan::PlanStatus::Executing;
    plan.steps = vec![archon_session::plan::PlanStep {
        number: 1,
        description: "finish approved work".into(),
        affected_files: Vec::new(),
        status: archon_session::plan::PlanStepStatus::Pending,
        blocked_by: Vec::new(),
        required_evidence: Vec::new(),
        task_id: None,
    }];
    plans.save_plan(session_id, &plan).unwrap();

    let verdict = turn_finalization_verdict_with_plan_db(
        world_model.path(),
        &session_database,
        session_id,
        "",
    );

    assert!(matches!(
        verdict,
        archon_core::agent::TurnFinalizationVerdict::Blocked { .. }
    ));
}

#[test]
fn passed_world_model_outcome_cannot_complete_a_durable_plan_task() {
    let (world_model, session_database, plans, session_id) = pending_durable_world_model_plan();
    let record = blocking_turn_record(session_id, "world-model-outcome-action");
    remember_active_guardrail(&record);
    let passed = verification_for(&record, archon_world_model::VerificationStatus::Passed);
    archon_world_model::guardrail::append_verification_outcome(world_model.path(), &passed).unwrap();

    let verdict = turn_finalization_verdict_with_plan_db(
        world_model.path(),
        &session_database,
        session_id,
        &record.action.action_id,
    );

    assert!(matches!(
        verdict,
        archon_core::agent::TurnFinalizationVerdict::Blocked { .. }
    ));
    assert_eq!(plans.load_plan_tasks(session_id).unwrap()[0].status, "Pending");
    clear_active_guardrail(session_id, &record.action.action_id);
}

fn pending_durable_world_model_plan(
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    archon_session::plan::PlanStore,
    &'static str,
) {
    let world_model = tempfile::tempdir().unwrap();
    let session_database = world_model.path().join("runtime-session.db");
    let session_id = "world-model-outcome-session";
    let store = archon_session::storage::SessionStore::open(&session_database).unwrap();
    let plans = archon_session::plan::PlanStore::new(store.db()).unwrap();
    let mut plan = archon_session::plan::PlanDocument::new("world-model-outcome-plan", "Plan");
    plan.session_id = Some(session_id.into());
    plan.status = archon_session::plan::PlanStatus::Approved;
    plan.approval = Some(noninteractive_approval());
    plan.steps = vec![durable_pending_test_step()];
    let authority = plans
        .bootstrap_approval_authority(session_id, [0xA5; 32])
        .unwrap();
    let approval = plan.approval.clone().unwrap();
    plans
        .save_terminal_plan_with_approval_and_tasks(
            &authority,
            session_id,
            &plan,
            &archon_session::plan::PlanApprovalRecord {
                plan_id: plan.id.clone(),
                session_id: session_id.into(),
                approval,
            },
            &[archon_session::plan::PersistedPlanTask {
                task_id: "world-model-outcome-task".into(),
                plan_id: plan.id.clone(),
                plan_step: 1,
                description: "run verified tests".into(),
                status: "Pending".into(),
                blocked_by: Vec::new(),
                required_evidence: vec![archon_completion::RequiredEvidenceKind::Tests],
                updated_at: "2026-08-15T00:00:00Z".into(),
            }],
        )
        .unwrap();
    (world_model, session_database, plans, session_id)
}

fn durable_pending_test_step() -> archon_session::plan::PlanStep {
    archon_session::plan::PlanStep {
        number: 1,
        description: "run verified tests".into(),
        affected_files: Vec::new(),
        status: archon_session::plan::PlanStepStatus::Pending,
        blocked_by: Vec::new(),
        required_evidence: vec![archon_completion::RequiredEvidenceKind::Tests],
        task_id: Some("world-model-outcome-task".into()),
    }
}

fn noninteractive_approval() -> archon_session::plan::PlanApproval {
    archon_session::plan::PlanApproval {
        decision: archon_session::plan::PlanApprovalDecision::Approve,
        source: archon_session::plan::PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    }
}

fn blocking_turn_record(session_id: &str, action_id: &str) -> RuntimeGuardrailRecord {
    let mut record = runtime_guardrail_record(session_id, action_id);
    record.decision.mode = archon_world_model::WorldGuardrailMode::Guarded;
    record.decision.allowed_to_finalize = false;
    record.decision.required_actions = vec![archon_world_model::GuardrailRequiredAction::RunTests];
    record.action.verification_plan =
        verification_plan_for_decision(&record.action.action_id, &record.decision);
    record
}

fn verification_for(
    record: &RuntimeGuardrailRecord,
    status: archon_world_model::VerificationStatus,
) -> archon_world_model::VerificationOutcome {
    archon_world_model::VerificationOutcome {
        requirement_id: record.action.verification_plan[0].requirement_id.clone(),
        action_id: record.action.action_id.clone(),
        kind: archon_world_model::VerificationKind::UnitTests,
        status,
        idempotency_key: format!("verification:{}:{status:?}", record.action.action_id),
        ..archon_world_model::VerificationOutcome::default()
    }
}
