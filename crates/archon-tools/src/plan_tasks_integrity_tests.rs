use super::test_support::{five_step_plan, plan_with_preassigned_task, test_store};
use super::*;
use crate::plan_tasks::test_plan_approval_authority;

#[test]
fn save_plan_task_rejects_existing_key_without_overwrite() {
    let store = test_store();
    let original = PersistedPlanTask {
        task_id: "immutable-task".into(),
        plan_id: "immutable-plan".into(),
        plan_step: 1,
        description: "original description".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    let replacement = PersistedPlanTask {
        description: "must not overwrite".into(),
        ..original.clone()
    };

    store
        .save_plan_task_fixture("immutable-session", &original)
        .unwrap();
    let error = store
        .save_plan_task_fixture("immutable-session", &replacement)
        .expect_err("public initial save must reject an existing task ID");

    assert!(error.to_string().contains("plan_tasks"));
    assert_eq!(
        store.load_plan_tasks("immutable-session").unwrap(),
        vec![original]
    );
}

#[test]
fn concurrent_materializations_with_same_durable_id_allow_one_without_overwrite() {
    use std::sync::{Arc, Barrier};

    let store = test_store();
    let first_manager = Arc::new(TaskManager::new());
    let second_manager = Arc::new(TaskManager::new());
    let barrier = Arc::new(Barrier::new(3));
    let _barrier_reset =
        set_materialization_barrier_for_test("shared-session".into(), Some(Arc::clone(&barrier)));

    let first_store = store.clone();
    let first_manager_for_thread = Arc::clone(&first_manager);
    let first = std::thread::spawn(move || {
        let mut plan = plan_with_preassigned_task("first-plan", "shared-task-id", "first winner");
        materialize_plan_tasks(
            &first_manager_for_thread,
            &first_store,
            &test_plan_approval_authority(&first_store, "shared-session"),
            "shared-session",
            &mut plan,
        )
    });
    let second_store = store.clone();
    let second_manager_for_thread = Arc::clone(&second_manager);
    let second = std::thread::spawn(move || {
        let mut plan = plan_with_preassigned_task("second-plan", "shared-task-id", "second winner");
        materialize_plan_tasks(
            &second_manager_for_thread,
            &second_store,
            &test_plan_approval_authority(&second_store, "shared-session"),
            "shared-session",
            &mut plan,
        )
    });
    barrier.wait();
    let first = first.join().expect("first materializer thread");
    let second = second.join().expect("second materializer thread");

    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let loser = if first.is_err() {
        &first_manager
    } else {
        &second_manager
    };
    let error = first.err().or_else(|| second.err()).expect("one collision");
    assert!(
        error.contains("plan_tasks") || error.contains("durable plan task"),
        "unexpected collision error: {error}"
    );
    let stored = store.load_plan_tasks("shared-session").unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].task_id, "shared-task-id");
    assert!(matches!(
        stored[0].description.as_str(),
        "first winner" | "second winner"
    ));
    assert!(loser.get_task("shared-task-id").is_none());
}

#[test]
fn prepared_plan_task_publication_rejects_foreign_authority() {
    let canonical_store = test_store();
    let foreign_store = test_store();
    let manager = TaskManager::new();
    let session_id = "forged-publication";
    let mut plan = five_step_plan();
    let infos = build_plan_task_infos(session_id, &mut plan).unwrap();
    let foreign_authority = test_plan_approval_authority(&foreign_store, session_id);

    let error = match manager.prepare_plan_task_installation(
        &foreign_authority,
        session_id,
        canonical_store,
        infos,
    ) {
        Ok(_) => panic!("foreign authority must not prepare task publication"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("approval authority"));
    assert!(manager.list_tasks().is_empty());
}

#[test]
fn prepared_plan_task_rehydration_rejects_foreign_authority() {
    let canonical_store = test_store();
    let foreign_store = test_store();
    let source = TaskManager::new();
    let session_id = "forged-rehydration";
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &source,
        &canonical_store,
        &test_plan_approval_authority(&canonical_store, session_id),
        session_id,
        &mut plan,
    )
    .unwrap();
    let durable = canonical_store.load_plan_tasks(session_id).unwrap();
    let infos = durable
        .iter()
        .map(|task| test_task_info_from_persisted(session_id, task))
        .collect::<Vec<_>>();
    let restarted = TaskManager::new();
    let foreign_authority = test_plan_approval_authority(&foreign_store, session_id);

    let error = match restarted.prepare_plan_task_rehydration(
        &foreign_authority,
        session_id,
        canonical_store,
        infos,
    ) {
        Ok(_) => panic!("foreign authority must not prepare task rehydration"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("approval authority"));
    assert!(ids.iter().all(|id| restarted.get_task(id).is_none()));
}

#[test]
fn rehydration_rejects_same_plan_task_id_with_status_or_metadata_mismatch() {
    let store = test_store();
    let source_manager = TaskManager::new();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &source_manager,
        &store,
        &test_plan_approval_authority(&store, "rehydrate-mismatch"),
        "rehydrate-mismatch",
        &mut plan,
    )
    .unwrap();
    source_manager
        .set_status_checked_with_evidence_ids(&ids[0], TaskStatus::Running, "", &[])
        .unwrap();
    let durable = store
        .load_plan_tasks("rehydrate-mismatch")
        .unwrap()
        .into_iter()
        .find(|task| task.task_id == ids[0])
        .unwrap();

    let status_manager = TaskManager::new();
    let mut wrong_status = test_task_info_from_persisted("rehydrate-mismatch", &durable);
    wrong_status.status = TaskStatus::Pending;
    status_manager.restore_plan_task(wrong_status).unwrap();
    let status_error = rehydrate_plan_tasks(
        &status_manager,
        &store,
        &test_plan_approval_authority(&store, "rehydrate-mismatch"),
        "rehydrate-mismatch",
    )
    .expect_err("same plan identity with a different status is corruption");
    assert!(status_error.contains("canonical mismatch"));
    assert_eq!(
        status_manager.get_task(&ids[0]).unwrap().status,
        TaskStatus::Pending
    );

    let metadata_manager = TaskManager::new();
    let mut wrong_metadata = test_task_info_from_persisted("rehydrate-mismatch", &durable);
    wrong_metadata
        .metadata
        .as_mut()
        .unwrap()
        .blocked_by
        .push("forged".into());
    metadata_manager.restore_plan_task(wrong_metadata).unwrap();
    let metadata_error = rehydrate_plan_tasks(
        &metadata_manager,
        &store,
        &test_plan_approval_authority(&store, "rehydrate-mismatch"),
        "rehydrate-mismatch",
    )
    .expect_err("same plan identity with different metadata is corruption");
    assert!(metadata_error.contains("canonical mismatch"));
    assert_eq!(
        metadata_manager
            .get_task(&ids[0])
            .unwrap()
            .metadata
            .unwrap()
            .blocked_by,
        vec!["forged".to_string()]
    );
}

#[test]
fn plan_task_description_updates_are_rejected_before_status_changes() {
    let manager = TaskManager::new();
    let store = test_store();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "description-immutable"),
        "description-immutable",
        &mut plan,
    )
    .unwrap();

    let error = manager
        .update_task(&ids[0], Some("forbidden rewrite"))
        .unwrap_err();
    assert!(error.to_string().contains("plan-linked task descriptions"));
    assert_eq!(manager.get_task(&ids[0]).unwrap().description, "step 1");
    let manual = manager.create_task("manual description");
    manager
        .update_task(&manual, Some("updated manual description"))
        .unwrap();
    assert_eq!(
        manager.get_task(&manual).unwrap().description,
        "updated manual description"
    );
}

#[test]
fn rehydrated_plan_tasks_retain_immutable_descriptions() {
    let source_manager = TaskManager::new();
    let store = test_store();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &source_manager,
        &store,
        &test_plan_approval_authority(&store, "description-immutable-restart"),
        "description-immutable-restart",
        &mut plan,
    )
    .unwrap();
    let restarted_manager = TaskManager::new();

    assert_eq!(
        rehydrate_plan_tasks(
            &restarted_manager,
            &store,
            &test_plan_approval_authority(&store, "description-immutable-restart"),
            "description-immutable-restart"
        )
        .unwrap(),
        ids.len()
    );
    let error = restarted_manager
        .update_task(&ids[0], Some("forbidden after restart"))
        .unwrap_err();
    assert!(error.to_string().contains("plan-linked task descriptions"));
    assert_eq!(
        restarted_manager.get_task(&ids[0]).unwrap().description,
        "step 1"
    );
}

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
