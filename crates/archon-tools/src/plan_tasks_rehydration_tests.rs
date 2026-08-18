use super::*;
use crate::plan_tasks::test_plan_approval_authority;
use archon_completion::RequiredEvidenceKind;
use archon_session::storage::SessionStore;

fn test_task_info_from_persisted(session_id: &str, task: &PersistedPlanTask) -> TaskInfo {
    TaskInfo {
        id: task.task_id.clone(),
        description: task.description.clone(),
        status: parse_status(&task.status).unwrap(),
        created_at: Utc::now(),
        completed_at: None,
        output: String::new(),
        cost: 0.0,
        agent_id: None,
        board_item_id: None,
        metadata: Some(PlanTaskMetadata {
            session_id: session_id.into(),
            plan_id: task.plan_id.clone(),
            plan_step: task.plan_step,
            blocked_by: task.blocked_by.clone(),
            required_evidence: task.required_evidence.clone(),
        }),
    }
}

fn corrupt_plan_for_test(db: &cozo::DbInstance, session_id: &str, plan: &PlanDocument) {
    let json = plan.to_json();
    let mut params = std::collections::BTreeMap::new();
    params.insert("session_id".to_string(), cozo::DataValue::from(session_id));
    params.insert(
        "plan_id".to_string(),
        cozo::DataValue::from(plan.id.as_str()),
    );
    params.insert(
        "plan_json".to_string(),
        cozo::DataValue::from(json.as_str()),
    );
    params.insert("updated_at".to_string(), cozo::DataValue::from("corrupt"));
    db
        .run_script(
            "?[session_id, plan_id, plan_json, updated_at] <- [[$session_id, $plan_id, $plan_json, $updated_at]]
             :put plans {session_id, plan_id => plan_json, updated_at}",
            params,
            cozo::ScriptMutability::Mutable,
        )
        .unwrap();
}

fn corrupt_plan_task_for_test(db: &cozo::DbInstance, session_id: &str, task: &PersistedPlanTask) {
    let json = serde_json::to_string(task).unwrap();
    let mut params = std::collections::BTreeMap::new();
    params.insert("session_id".to_string(), cozo::DataValue::from(session_id));
    params.insert(
        "task_id".to_string(),
        cozo::DataValue::from(task.task_id.as_str()),
    );
    params.insert(
        "plan_id".to_string(),
        cozo::DataValue::from(task.plan_id.as_str()),
    );
    params.insert(
        "plan_step".to_string(),
        cozo::DataValue::from(i64::from(task.plan_step)),
    );
    params.insert(
        "task_json".to_string(),
        cozo::DataValue::from(json.as_str()),
    );
    params.insert(
        "updated_at".to_string(),
        cozo::DataValue::from(task.updated_at.as_str()),
    );
    db
        .run_script(
            "?[session_id, task_id, plan_id, plan_step, task_json, updated_at] <- [[$session_id, $task_id, $plan_id, $plan_step, $task_json, $updated_at]]
             :put plan_tasks {session_id, task_id => plan_id, plan_step, task_json, updated_at}",
            params,
            cozo::ScriptMutability::Mutable,
        )
        .unwrap();
}

fn five_step_plan() -> PlanDocument {
    let mut plan = PlanDocument::new("plan-five", "Five-step approval plan");
    plan.status = PlanStatus::Approved;
    plan.approval = Some(archon_session::plan::PlanApproval {
        decision: archon_session::plan::PlanApprovalDecision::Approve,
        source: archon_session::plan::PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    });
    plan.steps = (1..=5)
        .map(|number| archon_session::plan::PlanStep {
            number,
            description: format!("step {number}"),
            affected_files: vec![],
            status: PlanStepStatus::Pending,
            blocked_by: if number == 1 {
                vec![]
            } else {
                vec![number - 1]
            },
            required_evidence: if number == 3 {
                vec![RequiredEvidenceKind::Tests]
            } else {
                vec![]
            },
            task_id: None,
        })
        .collect();
    plan
}

fn test_store() -> (cozo::DbInstance, PlanStore) {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let store = PlanStore::new(&db).unwrap();
    (db, store)
}

#[test]
fn empty_task_rows_reject_older_materialized_plan_even_when_newer_draft_exists() {
    let (_db, store) = test_store();
    let session_id = "empty-rows-with-newer-draft";
    let mut materialized = five_step_plan();
    materialized.id = "older-materialized".into();
    materialized.status = PlanStatus::Approved;
    for step in &mut materialized.steps {
        step.task_id = Some(format!("materialized-task-{}", step.number));
    }
    store.save_plan(session_id, &materialized).unwrap();

    let mut newer_draft = PlanDocument::new("newer-draft", "Unrelated newer draft");
    newer_draft.steps = vec![];
    store.save_plan(session_id, &newer_draft).unwrap();

    let restarted = TaskManager::new();
    let error = rehydrate_plan_tasks(
        &restarted,
        &store,
        &test_plan_approval_authority(&store, session_id),
        session_id,
    )
    .expect_err("missing durable rows for a materialized plan must fail closed");
    assert!(error.contains("materialized step count"), "{error}");
    assert_eq!(
        restarted.installed_plan_task_count_for_session_for_test(session_id),
        0
    );
}

#[test]
fn rehydration_rejects_task_rows_for_draft_plan() {
    let (db, store) = test_store();
    let source = TaskManager::new();
    let session_id = "draft-with-durable-tasks";
    let mut plan = five_step_plan();
    plan.status = PlanStatus::Approved;
    let ids = materialize_plan_tasks(
        &source,
        &store,
        &test_plan_approval_authority(&store, session_id),
        session_id,
        &mut plan,
    )
    .unwrap();
    plan.status = PlanStatus::Draft;
    corrupt_plan_for_test(&db, session_id, &plan);

    let restarted = TaskManager::new();
    let error = rehydrate_plan_tasks(
        &restarted,
        &store,
        &test_plan_approval_authority(&store, session_id),
        session_id,
    )
    .expect_err("draft plans must never publish executable tasks");
    assert!(error.contains("active materialized plan"), "{error}");
    assert!(ids.iter().all(|id| restarted.get_task(id).is_none()));
}

#[test]
fn reopened_plan_store_accepts_existing_attachment_and_rehydrates_durable_tasks() {
    let temp = tempfile::tempdir().unwrap();
    let session_db = temp.path().join("session.db");
    let session_id = format!("reopen-physical-store-{}", uuid::Uuid::new_v4());
    let first_session_store = SessionStore::open(&session_db).unwrap();
    let first_store = PlanStore::new(first_session_store.db()).unwrap();
    let source = TaskManager::new();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &source,
        &first_store,
        &test_plan_approval_authority(&first_store, &session_id),
        &session_id,
        &mut plan,
    )
    .unwrap();
    let manager = TaskManager::new();
    manager
        .attach_plan_store(first_store.clone(), &session_id)
        .unwrap();

    drop(first_store);
    drop(first_session_store);

    let reopened_session_store = SessionStore::open(&session_db).unwrap();
    let reopened_store = PlanStore::new(reopened_session_store.db()).unwrap();
    assert_eq!(
        rehydrate_plan_tasks(
            &manager,
            &reopened_store,
            &test_plan_approval_authority(&reopened_store, &session_id),
            &session_id
        )
        .unwrap(),
        ids.len(),
        "an existing manager attachment must accept a reopened handle to the same database"
    );
    assert!(ids.iter().all(|id| manager.get_task(id).is_some()));
}

#[test]
fn rehydration_rejects_different_existing_store_without_mutation() {
    let (_first_db, first_store) = test_store();
    let (_second_db, second_store) = test_store();
    let session_id = "different-existing-store";
    let manager = TaskManager::new();
    manager
        .attach_plan_store(first_store.clone(), session_id)
        .unwrap();
    let source = TaskManager::new();
    let mut plan = five_step_plan();
    plan.status = PlanStatus::Approved;
    let ids = materialize_plan_tasks(
        &source,
        &second_store,
        &test_plan_approval_authority(&second_store, session_id),
        session_id,
        &mut plan,
    )
    .unwrap();

    let error = rehydrate_plan_tasks(
        &manager,
        &second_store,
        &test_plan_approval_authority(&second_store, session_id),
        session_id,
    )
    .expect_err("rehydration must not mix tasks and persistence stores");
    assert!(error.contains("different plan store"), "{error}");
    assert!(ids.iter().all(|id| manager.get_task(id).is_none()));
    manager
        .attach_plan_store(first_store.clone(), session_id)
        .expect("original attachment must remain");
    assert!(manager.attach_plan_store(second_store, session_id).is_err());
}

#[test]
fn rehydration_restores_multiple_materialized_plans_for_one_session() {
    let (_db, store) = test_store();
    let source = TaskManager::new();
    let session_id = "multiple-materialized-plans";
    let mut first = five_step_plan();
    first.id = "first-materialized".into();
    first.status = PlanStatus::Approved;
    let first_ids = materialize_plan_tasks(
        &source,
        &store,
        &test_plan_approval_authority(&store, session_id),
        session_id,
        &mut first,
    )
    .unwrap();
    let mut second = five_step_plan();
    second.id = "second-materialized".into();
    second.status = PlanStatus::Approved;
    let second_ids = materialize_plan_tasks(
        &source,
        &store,
        &test_plan_approval_authority(&store, session_id),
        session_id,
        &mut second,
    )
    .unwrap();

    let restarted = TaskManager::new();
    assert_eq!(
        rehydrate_plan_tasks(
            &restarted,
            &store,
            &test_plan_approval_authority(&store, session_id),
            session_id
        )
        .unwrap(),
        first_ids.len() + second_ids.len()
    );
    assert!(
        first_ids
            .iter()
            .chain(second_ids.iter())
            .all(|id| restarted.get_task(id).is_some())
    );
}

#[test]
fn failed_materialization_preserves_existing_store_attachment() {
    let (_first_db, first_store) = test_store();
    let (_second_db, second_store) = test_store();
    let session_id = "materialization-existing-store";
    let manager = TaskManager::new();
    manager
        .attach_plan_store(first_store.clone(), session_id)
        .unwrap();
    let mut plan = five_step_plan();

    let error = materialize_plan_tasks(
        &manager,
        &second_store,
        &test_plan_approval_authority(&second_store, session_id),
        session_id,
        &mut plan,
    )
    .expect_err("different store attachment must fail before persistence");
    assert!(error.contains("different plan store"), "{error}");
    assert!(manager.list_tasks().is_empty());
    assert!(second_store.load_plan_tasks(session_id).unwrap().is_empty());
    manager
        .attach_plan_store(first_store, session_id)
        .expect("original attachment must remain");
}

#[test]
fn materialization_rejects_nonapproved_plan_before_mutation() {
    let (_db, store) = test_store();
    let manager = TaskManager::new();
    let mut draft = five_step_plan();
    draft.status = PlanStatus::Draft;

    let error = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "draft-materialization"),
        "draft-materialization",
        &mut draft,
    )
    .expect_err("draft plan must not create executable tasks");
    assert!(error.contains("cannot materialize tasks"), "{error}");
    assert!(draft.steps.iter().all(|step| step.task_id.is_none()));
    assert!(
        store
            .load_plan_tasks("draft-materialization")
            .unwrap()
            .is_empty()
    );
    assert!(manager.list_tasks().is_empty());
}

#[test]
fn rehydration_rejects_corrupt_durable_status_without_attaching_store() {
    let manager = TaskManager::new();
    let (db, store) = test_store();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "corrupt"),
        "corrupt",
        &mut plan,
    )
    .unwrap();
    let mut corrupted = store.load_plan_tasks("corrupt").unwrap();
    corrupted[0].status = "Corrupt".into();
    corrupt_plan_task_for_test(&db, "corrupt", &corrupted[0]);

    let rehydrated = TaskManager::new();
    let error = rehydrate_plan_tasks(
        &rehydrated,
        &store,
        &test_plan_approval_authority(&store, "corrupt"),
        "corrupt",
    )
    .unwrap_err();
    assert!(error.contains("unknown persisted task status: Corrupt"));
    assert!(rehydrated.get_task(&ids[0]).is_none());

    let inserted = rehydrated
        .insert_plan_task(TaskInfo {
            id: "unattached".into(),
            description: "must not persist".into(),
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            completed_at: None,
            output: String::new(),
            cost: 0.0,
            agent_id: None,
            board_item_id: None,
            metadata: Some(PlanTaskMetadata {
                session_id: "corrupt".into(),
                plan_id: plan.id,
                plan_step: 1,
                blocked_by: vec![],
                required_evidence: vec![],
            }),
        })
        .unwrap_err();
    assert!(matches!(
        inserted,
        crate::task_manager::TaskTransitionError::Persistence(_)
    ));
    assert!(rehydrated.get_task("unattached").is_none());
}

#[path = "plan_tasks_rehydration_integrity_tests.rs"]
mod integrity_tests;
