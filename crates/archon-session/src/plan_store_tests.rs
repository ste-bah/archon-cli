use cozo::DbInstance;

use super::*;

fn test_db() -> DbInstance {
    DbInstance::new("mem", "", "").expect("in-memory db")
}

fn test_authority(store: &PlanStore, session_id: &str) -> PlanApprovalAuthority {
    store
        .bootstrap_approval_authority_for_test(session_id)
        .expect("test authority")
}

#[path = "plan_store_authority_tests.rs"]
mod authority_tests;

#[path = "plan_store_materialization_validation_tests.rs"]
mod materialization_validation_tests;

fn approve_for_materialization(store: &PlanStore, session_id: &str, plan: &mut PlanDocument) {
    let approval = PlanApproval {
        decision: PlanApprovalDecision::Approve,
        source: PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    };
    plan.approval = Some(approval.clone());
    let record = PlanApprovalRecord {
        plan_id: plan.id.clone(),
        session_id: session_id.into(),
        approval,
    };
    store
        .save_terminal_plan_with_approval(
            &test_authority(store, session_id),
            session_id,
            plan,
            &record,
        )
        .expect("persist approved canonical plan");
}

fn assert_materialized_plan_unchanged(
    store: &PlanStore,
    session_id: &str,
    plan: &PlanDocument,
    task: &PersistedPlanTask,
) {
    assert_eq!(
        store.load_plan_tasks(session_id).unwrap(),
        vec![task.clone()]
    );
    assert_eq!(
        store
            .load_plan(session_id, &plan.id)
            .unwrap()
            .unwrap()
            .to_json(),
        plan.to_json()
    );
}

#[test]
fn approval_events_roundtrip_in_durable_ledger() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let record = PlanApprovalRecord {
        plan_id: "plan-approval".into(),
        session_id: "session-approval".into(),
        approval: PlanApproval {
            decision: PlanApprovalDecision::ApproveAcceptEdits,
            source: PlanApprovalSource::NonInteractive,
            decided_at: "2026-08-14T00:00:00Z".into(),
            user_edited: true,
        },
    };
    store
        .record_approval_event_for_test(&record)
        .expect("record");
    let duplicate = store.record_approval_event_for_test(&record);
    assert!(
        duplicate.is_err(),
        "approval ledger must not overwrite events"
    );
    assert_eq!(
        store
            .load_approval_events("session-approval", "plan-approval")
            .expect("load"),
        vec![record]
    );
}
#[test]
fn terminal_plan_and_approval_are_persisted_atomically() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let mut plan = PlanDocument::new("terminal-plan", "Terminal Plan");
    plan.status = PlanStatus::Approved;
    let record = PlanApprovalRecord {
        plan_id: plan.id.clone(),
        session_id: "terminal-session".into(),
        approval: PlanApproval {
            decision: PlanApprovalDecision::Approve,
            source: PlanApprovalSource::Interactive,
            decided_at: "2026-08-14T00:00:00Z".into(),
            user_edited: false,
        },
    };
    plan.approval = Some(record.approval.clone());

    store
        .save_terminal_plan_with_approval(
            &test_authority(&store, "terminal-session"),
            "terminal-session",
            &plan,
            &record,
        )
        .expect("atomic save");

    let loaded = store
        .load_plan("terminal-session", "terminal-plan")
        .expect("load")
        .expect("terminal plan");
    assert_eq!(loaded.status, PlanStatus::Approved);
    assert_eq!(loaded.approval, Some(record.approval.clone()));
    assert_eq!(
        store
            .load_approval_events("terminal-session", "terminal-plan")
            .expect("ledger"),
        vec![record]
    );
}
#[test]
fn terminal_plan_and_approval_roll_back_together_on_second_write_failure() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let mut plan = PlanDocument::new("rollback-plan", "Rollback Plan");
    plan.status = PlanStatus::Approved;
    let record = PlanApprovalRecord {
        plan_id: plan.id.clone(),
        session_id: "rollback-session".into(),
        approval: PlanApproval {
            decision: PlanApprovalDecision::Approve,
            source: PlanApprovalSource::Interactive,
            decided_at: "2026-08-14T00:00:00Z".into(),
            user_edited: false,
        },
    };
    plan.approval = Some(record.approval.clone());
    store
        .record_approval_event_for_test(&record)
        .expect("seed colliding immutable ledger event");

    let error = store
        .save_terminal_plan_with_approval(
            &test_authority(&store, "rollback-session"),
            "rollback-session",
            &plan,
            &record,
        )
        .expect_err("duplicate ledger event must abort the transaction");
    assert!(error.to_string().contains("plan_approval_events"));
    assert!(
        store
            .load_plan("rollback-session", "rollback-plan")
            .expect("load")
            .is_none(),
        "failed terminal save must not leave a plan document"
    );
    assert_eq!(
        store
            .load_approval_events("rollback-session", "rollback-plan")
            .expect("ledger")
            .len(),
        1,
        "rollback must leave only the original event"
    );
}
#[test]
fn terminal_plan_approval_and_tasks_roll_back_on_task_collision() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "terminal-task-collision";
    let mut draft = PlanDocument::new("terminal-task-plan", "Terminal task collision");
    draft
        .steps
        .push(step(1, "must remain draft", PlanStepStatus::Pending));
    store.save_plan(session_id, &draft).expect("save draft");
    let existing_task = PersistedPlanTask {
        task_id: "terminal-task-id".into(),
        plan_id: "unrelated-plan".into(),
        plan_step: 4,
        description: "existing durable task".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    store
        .save_plan_task_fixture(session_id, &existing_task)
        .expect("seed durable collision");

    let mut terminal = draft.clone();
    terminal.status = PlanStatus::Approved;
    terminal.steps[0].task_id = Some(existing_task.task_id.clone());
    terminal.steps[0].description = "must not persist".into();
    let record = PlanApprovalRecord {
        plan_id: terminal.id.clone(),
        session_id: session_id.into(),
        approval: PlanApproval {
            decision: PlanApprovalDecision::Approve,
            source: PlanApprovalSource::Interactive,
            decided_at: "2026-08-15T00:00:01Z".into(),
            user_edited: false,
        },
    };
    terminal.approval = Some(record.approval.clone());
    let colliding_task = PersistedPlanTask {
        plan_id: terminal.id.clone(),
        plan_step: 1,
        description: "must not persist".into(),
        ..existing_task.clone()
    };

    let error = store
        .save_terminal_plan_with_approval_and_tasks(
            &test_authority(&store, session_id),
            session_id,
            &terminal,
            &record,
            &[colliding_task],
        )
        .expect_err("task collision must abort the terminal transaction");

    assert!(
        error.to_string().contains("relation 'plan_tasks'"),
        "unexpected collision error: {error}"
    );
    let preserved = store
        .load_plan(session_id, &draft.id)
        .unwrap()
        .expect("draft survives failed terminal write");
    assert_eq!(preserved.status, PlanStatus::Draft);
    assert!(preserved.approval.is_none());
    assert!(
        store
            .load_approval_events(session_id, &terminal.id)
            .unwrap()
            .is_empty(),
        "failed terminal write must not append approval ledger evidence"
    );
    assert_eq!(
        store.load_plan_tasks(session_id).unwrap(),
        vec![existing_task],
        "failed terminal write must not overwrite the existing task"
    );
}
#[test]
fn save_plan_with_tasks_rejects_second_generation_for_same_plan() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "public-materialization-generation";
    let mut first = PlanDocument::new("public-materialization-plan", "Public materialization");
    first.status = PlanStatus::Approved;
    first.steps.push(PlanStep {
        task_id: Some("first-task".into()),
        ..step(1, "first", PlanStepStatus::Pending)
    });
    approve_for_materialization(&store, session_id, &mut first);
    let first_task = PersistedPlanTask {
        task_id: "first-task".into(),
        plan_id: first.id.clone(),
        plan_step: 1,
        description: "first".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    store
        .save_plan_with_tasks(
            &test_authority(&store, session_id),
            session_id,
            &first,
            std::slice::from_ref(&first_task),
        )
        .expect("first generation");

    let mut second = first.clone();
    second.steps[0].task_id = Some("second-task".into());
    let second_task = PersistedPlanTask {
        task_id: "second-task".into(),
        updated_at: "2026-08-15T00:00:01Z".into(),
        ..first_task
    };
    let error = store
        .save_plan_with_tasks(
            &test_authority(&store, session_id),
            session_id,
            &second,
            &[second_task],
        )
        .expect_err("a second generation must not overwrite canonical tasks");

    assert!(error.to_string().contains("materialization"));
    assert_eq!(
        store
            .load_plan(session_id, &first.id)
            .unwrap()
            .unwrap()
            .to_json(),
        first.to_json()
    );
    assert_eq!(store.load_plan_tasks(session_id).unwrap().len(), 1);
    let overwrite = store
        .save_plan(session_id, &second)
        .expect_err("public plan write must not overwrite a materialized generation");
    assert!(
        overwrite
            .to_string()
            .contains("materialized canonical plan")
    );
}
#[test]
fn legacy_coherent_plan_tasks_are_claimed_without_rewrite() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "legacy-materialization-generation";
    let mut plan = PlanDocument::new("legacy-materialization-plan", "Legacy materialization");
    plan.status = PlanStatus::Approved;
    plan.steps.push(PlanStep {
        task_id: Some("legacy-task".into()),
        ..step(1, "legacy", PlanStepStatus::Pending)
    });
    approve_for_materialization(&store, session_id, &mut plan);
    let task = PersistedPlanTask {
        task_id: "legacy-task".into(),
        plan_id: plan.id.clone(),
        plan_step: 1,
        description: "legacy".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    store.save_plan(session_id, &plan).expect("legacy plan");
    store
        .save_plan_task_fixture(session_id, &task)
        .expect("legacy task");

    store
        .save_plan_with_tasks(
            &test_authority(&store, session_id),
            session_id,
            &plan,
            std::slice::from_ref(&task),
        )
        .expect("adopt coherent legacy generation");
    store
        .save_plan_with_tasks(
            &test_authority(&store, session_id),
            session_id,
            &plan,
            &[task],
        )
        .expect("claimed generation is idempotent");
    assert_eq!(store.load_plan_tasks(session_id).unwrap().len(), 1);
}
#[test]
fn raw_plan_task_status_mutator_is_test_only_and_rejects_all_writes() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let mut plan = PlanDocument::new("public-status-plan", "Public status update");
    plan.status = PlanStatus::Approved;
    plan.steps.push(PlanStep {
        task_id: Some("public-status-task".into()),
        ..step(1, "first", PlanStepStatus::Pending)
    });
    approve_for_materialization(&store, "public-status-session", &mut plan);
    let task = PersistedPlanTask {
        task_id: "public-status-task".into(),
        plan_id: plan.id.clone(),
        plan_step: 1,
        description: "first".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    store
        .save_plan_with_tasks(
            &test_authority(&store, "public-status-session"),
            "public-status-session",
            &plan,
            std::slice::from_ref(&task),
        )
        .expect("seed");

    let error = store
        .update_plan_task_status("public-status-session", "public-status-task", "Running")
        .expect_err("raw status mutation must remain unavailable");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_materialized_plan_unchanged(&store, "public-status-session", &plan, &task);
}
#[test]
fn raw_plan_task_status_mutator_rejects_invalid_status_without_mutating() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let mut plan = PlanDocument::new("public-status-rollback", "Public status rollback");
    plan.status = PlanStatus::Approved;
    plan.steps.push(PlanStep {
        task_id: Some("public-status-rollback-task".into()),
        ..step(1, "first", PlanStepStatus::Pending)
    });
    approve_for_materialization(&store, "public-status-rollback-session", &mut plan);
    let task = PersistedPlanTask {
        task_id: "public-status-rollback-task".into(),
        plan_id: plan.id.clone(),
        plan_step: 1,
        description: "first".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    store
        .save_plan_with_tasks(
            &test_authority(&store, "public-status-rollback-session"),
            "public-status-rollback-session",
            &plan,
            std::slice::from_ref(&task),
        )
        .expect("seed");

    let error = store
        .update_plan_task_status(
            "public-status-rollback-session",
            "public-status-rollback-task",
            "not-a-status",
        )
        .expect_err("raw status mutation must remain unavailable");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_materialized_plan_unchanged(&store, "public-status-rollback-session", &plan, &task);
}
fn step(number: u32, description: &str, status: PlanStepStatus) -> PlanStep {
    PlanStep {
        number,
        description: description.into(),
        affected_files: Vec::new(),
        status,
        blocked_by: Vec::new(),
        required_evidence: Vec::new(),
        task_id: None,
    }
}
