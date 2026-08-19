use archon_completion::RequiredEvidenceKind;
use archon_session::plan::PlanStatus;
use std::sync::Arc;

use super::*;
use crate::command::test_support::*;

fn step(number: u32, description: &str) -> PlanStep {
    PlanStep {
        number,
        description: description.to_string(),
        affected_files: vec![format!("src/{number}.rs")],
        status: PlanStepStatus::Complete,
        blocked_by: vec![number],
        required_evidence: vec![RequiredEvidenceKind::Tests],
        task_id: Some(format!("task-{number}")),
    }
}

#[test]
fn plan_emits_confirmation_textdelta() {
    let (mut ctx, mut rx) = make_bug_ctx();
    PlanHandler.execute(&mut ctx, &[]).unwrap();
    let events = drain_tui_events(&mut rx);
    assert!(
        matches!(events.as_slice(), [TuiEvent::TextDelta(message)] if message.contains("Plan mode enabled"))
    );
    assert!(matches!(
        ctx.pending_effect,
        Some(CommandEffect::EnterPlanMode { .. })
    ));
}

#[test]
fn draft_plan_ids_are_unique_and_safe_opaque_components() {
    let first = PlanHandler::draft_plan_id();
    let second = PlanHandler::draft_plan_id();

    assert_ne!(first, second, "immediate draft IDs must not collide");
    let tmp = tempfile::tempdir().unwrap();
    assert!(plan_file::plan_document_path(tmp.path(), &first).is_ok());
    assert!(plan_file::plan_document_path(tmp.path(), &second).is_ok());
}

#[test]
fn plan_open_spawns_editor_and_reports_path() {
    unsafe { std::env::set_var("EDITOR", "true") };
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(cozo::DbInstance::new("mem", "", "").unwrap());
    let (mut ctx, mut rx) = CtxBuilder::new()
        .with_working_dir(tmp.path().to_path_buf())
        .with_session_id("open-session".to_string())
        .with_cozo_db(db)
        .build();

    PlanHandler
        .execute(&mut ctx, &[String::from("open")])
        .unwrap();
    assert!(matches!(
        drain_tui_events(&mut rx).as_slice(),
        [TuiEvent::TextDelta(message)] if message.contains("Opened and saved plan document")
    ));
    assert!(matches!(
        ctx.pending_effect,
        Some(CommandEffect::SetActivePlanId(_))
    ));
}

#[test]
fn plan_open_rejects_unsafe_active_id_before_file_io() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(cozo::DbInstance::new("mem", "", "").unwrap());
    let (mut ctx, mut rx) = CtxBuilder::new()
        .with_working_dir(tmp.path().to_path_buf())
        .with_session_id("safe-session".to_string())
        .with_cozo_db(db)
        .with_plan_snapshot(PlanSnapshot {
            current_mode: PermissionMode::Plan,
            active_plan_id: Some("../../escape".to_string()),
        })
        .build();

    PlanHandler
        .execute(&mut ctx, &[String::from("open")])
        .unwrap();
    assert!(ctx.pending_effect.is_none());
    assert!(matches!(
        drain_tui_events(&mut rx).as_slice(),
        [TuiEvent::Error(message)] if message.contains("Invalid plan ID")
    ));
    assert!(!tmp.path().join("escape.md").exists());
}

#[test]
fn plan_open_creates_active_document_and_persists_editor_changes() {
    unsafe { std::env::set_var("EDITOR", "true") };
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(cozo::DbInstance::new("mem", "", "").unwrap());
    let session_id = "editor-session".to_string();
    let plan_id = "active-plan".to_string();
    let mut original = PlanDocument::new(&plan_id, "Original title");
    original.session_id = Some(session_id.clone());
    original.steps.push(step(1, "Keep the active ID"));
    PlanStore::new(&db)
        .unwrap()
        .save_plan(&session_id, &original)
        .unwrap();
    let document_path = plan_file::plan_document_path(tmp.path(), &plan_id).unwrap();
    std::fs::create_dir_all(document_path.parent().unwrap()).unwrap();
    std::fs::write(&document_path, "# Plan: Edited title\n\n## Steps\n\n1. Keep the active ID\n\n## Risks\n\n- Preserve structured data\n").unwrap();

    let (mut ctx, _rx) = CtxBuilder::new()
        .with_working_dir(tmp.path().to_path_buf())
        .with_session_id(session_id.clone())
        .with_cozo_db(Arc::clone(&db))
        .with_plan_snapshot(PlanSnapshot {
            current_mode: PermissionMode::Plan,
            active_plan_id: Some(plan_id.clone()),
        })
        .build();
    PlanHandler
        .execute(&mut ctx, &[String::from("open")])
        .unwrap();

    let saved = PlanStore::new(&db)
        .unwrap()
        .load_plan(&session_id, &plan_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.title, "Edited title");
    assert!(
        saved.user_edited,
        "successful editor reread must persist user_edited"
    );
    let saved_step = &saved.steps[0];
    let original_step = &original.steps[0];
    assert_eq!(saved_step.number, original_step.number);
    assert_eq!(saved_step.description, original_step.description);
    assert_eq!(saved_step.affected_files, original_step.affected_files);
    assert_eq!(saved_step.status, original_step.status);
    assert_eq!(saved_step.blocked_by, original_step.blocked_by);
    assert_eq!(
        saved_step.required_evidence,
        original_step.required_evidence
    );
    assert_eq!(saved_step.task_id, original_step.task_id);
    assert_eq!(saved.risks, vec!["Preserve structured data"]);
    assert!(
        matches!(ctx.pending_effect, Some(CommandEffect::SetActivePlanId(ref id)) if id == &plan_id)
    );
}

#[test]
fn parser_ignores_section_headers_in_risks_and_questions() {
    let prior = PlanDocument::new("plan", "Prior");
    let parsed = PlanHandler::parse_edited_document(
        "# Plan: Round trip\n\n## Steps\n\n1. One\n\n## Risks\n\n- Real risk\n\n## Questions\n\n- Real question\n",
        &prior,
    ).unwrap();
    assert_eq!(parsed.risks, vec!["Real risk"]);
    assert_eq!(parsed.questions, vec!["Real question"]);
}

#[test]
fn parser_preserves_all_metadata_only_for_unchanged_positional_steps() {
    let mut prior = PlanDocument::new("plan", "Prior");
    prior.steps = vec![step(1, "Keep"), step(2, "Change")];
    let parsed = PlanHandler::parse_edited_document(
        "# Plan: Prior\n\n## Steps\n\n1. Keep\n\n2. Changed\n",
        &prior,
    )
    .unwrap();
    let parsed_step = &parsed.steps[0];
    let prior_step = &prior.steps[0];
    assert_eq!(parsed_step.number, prior_step.number);
    assert_eq!(parsed_step.description, prior_step.description);
    assert_eq!(parsed_step.affected_files, prior_step.affected_files);
    assert_eq!(parsed_step.status, prior_step.status);
    assert_eq!(parsed_step.blocked_by, prior_step.blocked_by);
    assert_eq!(parsed_step.required_evidence, prior_step.required_evidence);
    assert_eq!(parsed_step.task_id, prior_step.task_id);
    assert_eq!(parsed.steps[1].status, PlanStepStatus::Pending);
    assert!(parsed.steps[1].affected_files.is_empty());
    assert!(parsed.steps[1].blocked_by.is_empty());
    assert!(parsed.steps[1].required_evidence.is_empty());
    assert!(parsed.steps[1].task_id.is_none());
}

#[test]
fn parser_resets_inserted_and_reordered_steps_without_metadata_inheritance() {
    let mut prior = PlanDocument::new("plan", "Prior");
    prior.steps = vec![step(1, "First"), step(2, "Second")];
    let inserted = PlanHandler::parse_edited_document(
        "# Plan: Prior\n\n## Steps\n\n1. Inserted\n\n2. First\n\n3. Second\n",
        &prior,
    )
    .unwrap();
    assert!(
        inserted
            .steps
            .iter()
            .all(|step| step.status == PlanStepStatus::Pending)
    );
    assert!(inserted.steps.iter().all(|step| step.task_id.is_none()));
    let reordered = PlanHandler::parse_edited_document(
        "# Plan: Prior\n\n## Steps\n\n2. Second\n\n1. First\n",
        &prior,
    )
    .unwrap();
    assert!(
        reordered
            .steps
            .iter()
            .all(|step| step.status == PlanStepStatus::Pending)
    );
    assert!(
        reordered
            .steps
            .iter()
            .all(|step| step.required_evidence.is_empty())
    );
}

#[test]
fn invalid_editor_document_retains_prior_structured_plan() {
    unsafe { std::env::set_var("EDITOR", "true") };
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(cozo::DbInstance::new("mem", "", "").unwrap());
    let session_id = "parse-failure-session".to_string();
    let plan_id = "protected-plan".to_string();
    let mut prior = PlanDocument::new(&plan_id, "Prior title");
    prior.steps.push(step(1, "Keep this step"));
    PlanStore::new(&db)
        .unwrap()
        .save_plan(&session_id, &prior)
        .unwrap();
    let path = plan_file::plan_document_path(tmp.path(), &plan_id).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "# Plan: Broken document\n\nNo steps heading\n").unwrap();
    let (mut ctx, mut rx) = CtxBuilder::new()
        .with_working_dir(tmp.path().to_path_buf())
        .with_session_id(session_id.clone())
        .with_cozo_db(Arc::clone(&db))
        .with_plan_snapshot(PlanSnapshot {
            current_mode: PermissionMode::Plan,
            active_plan_id: Some(plan_id.clone()),
        })
        .build();
    PlanHandler
        .execute(&mut ctx, &[String::from("open")])
        .unwrap();
    assert!(
        matches!(drain_tui_events(&mut rx).as_slice(), [TuiEvent::Error(message)] if message.contains("not saved"))
    );
    let saved = PlanStore::new(&db)
        .unwrap()
        .load_plan(&session_id, &plan_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.id, prior.id);
    assert_eq!(saved.title, prior.title);
    assert_eq!(saved.steps[0].description, prior.steps[0].description);
    assert_eq!(saved.steps[0].task_id, prior.steps[0].task_id);
    assert!(!saved.user_edited);
}

#[test]
fn completed_plan_remains_available_after_cozo_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let database_path = tmp.path().join("plans.db");
    let mut completed = PlanDocument::new("completed-plan", "Completed work");
    completed.status = PlanStatus::Completed;
    {
        let config = archon_cozo::CozoGuardConfig::for_db_path(&database_path);
        let db = archon_cozo::open_sqlite_guarded_instance(
            database_path.to_str().unwrap(),
            "plan reopen test: create",
            config,
        )
        .unwrap();
        PlanStore::new(db.db())
            .unwrap()
            .save_plan("reopen-session", &completed)
            .unwrap();
    }
    let config = archon_cozo::CozoGuardConfig::for_db_path(&database_path);
    let db = archon_cozo::open_sqlite_guarded_instance(
        database_path.to_str().unwrap(),
        "plan reopen test: reopen",
        config,
    )
    .unwrap();
    assert_eq!(
        PlanStore::new(db.db())
            .unwrap()
            .load_plan("reopen-session", "completed-plan")
            .unwrap()
            .unwrap()
            .status,
        PlanStatus::Completed
    );
}

#[test]
#[ignore = "Gate 5 smoke: execute manually against a temporary project"]
fn two_sessions_get_separate_audits_and_one_editable_plan_document() {
    let tmp = tempfile::tempdir().unwrap();
    let document = plan_file::plan_document_path(tmp.path(), "plan-one").unwrap();
    let first_audit = archon_core::plan_file::plan_audit_path(tmp.path(), "session-one").unwrap();
    let second_audit = archon_core::plan_file::plan_audit_path(tmp.path(), "session-two").unwrap();
    let plan = PlanDocument::new("plan-one", "Shared editable plan");
    plan_file::write_plan_document(&document, &plan).unwrap();
    archon_core::plan_file::append_plan_entry(&first_audit, "Write", &serde_json::json!({}))
        .unwrap();
    archon_core::plan_file::append_plan_entry(&second_audit, "Bash", &serde_json::json!({}))
        .unwrap();
    assert!(document.exists() && first_audit.exists() && second_audit.exists());
}

#[test]
fn plan_reads_active_document_and_flips_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let path = plan_file::plan_document_path(tmp.path(), "existing-plan").unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "# Plan: My plan\n\n## Steps\n\n1. First step\n").unwrap();
    let (mut ctx, mut rx) = CtxBuilder::new()
        .with_working_dir(tmp.path().to_path_buf())
        .with_plan_snapshot(PlanSnapshot {
            current_mode: PermissionMode::Default,
            active_plan_id: Some("existing-plan".to_string()),
        })
        .build();
    PlanHandler.execute(&mut ctx, &[]).unwrap();
    assert!(
        matches!(drain_tui_events(&mut rx).as_slice(), [TuiEvent::TextDelta(message)] if message.contains("1. First step"))
    );
    assert!(matches!(
        ctx.pending_effect,
        Some(CommandEffect::EnterPlanMode { .. })
    ));
}

#[test]
fn plan_off_stashes_default_effect() {
    let (mut ctx, mut rx) = make_bug_ctx();
    PlanHandler
        .execute(&mut ctx, &[String::from("off")])
        .unwrap();
    assert!(
        matches!(ctx.pending_effect, Some(CommandEffect::SetPermissionMode(ref mode)) if mode == "default")
    );
    assert!(
        matches!(drain_tui_events(&mut rx).as_slice(), [TuiEvent::TextDelta(message)] if message.contains("disabled"))
    );
}
