use super::*;

#[test]
fn rehydration_rejects_rows_that_disagree_with_the_canonical_plan() {
    fn assert_rejected(
        label: &str,
        mutate: impl FnOnce(&mut PlanDocument, &mut Vec<PersistedPlanTask>),
    ) {
        let (db, store) = test_store();
        let source = TaskManager::new();
        let session_id = format!("canonical-{label}");
        let mut plan = five_step_plan();
        materialize_plan_tasks(
            &source,
            &store,
            &test_plan_approval_authority(&store, &session_id),
            &session_id,
            &mut plan,
        )
        .unwrap();
        let mut tasks = store.load_plan_tasks(&session_id).unwrap();
        mutate(&mut plan, &mut tasks);
        corrupt_plan_for_test(&db, &session_id, &plan);
        for task in &tasks {
            corrupt_plan_task_for_test(&db, &session_id, task);
        }

        let restarted = TaskManager::new();
        let error = rehydrate_plan_tasks(
            &restarted,
            &store,
            &test_plan_approval_authority(&store, &session_id),
            &session_id,
        )
        .unwrap_err();
        assert!(error.contains("canonical plan"), "{label}: {error}");
        assert_eq!(
            restarted.installed_plan_task_count_for_session_for_test(&session_id),
            0,
            "{label} must not publish corrupt tasks"
        );
    }

    assert_rejected("task-id", |plan, _| {
        plan.steps[0].task_id = Some("forged-task-id".into());
    });
    assert_rejected("description", |plan, _| {
        plan.steps[0].description = "forged description".into();
    });
    assert_rejected("dependencies", |plan, _| {
        plan.steps[1].blocked_by.clear();
    });
    assert_rejected("evidence", |plan, _| {
        plan.steps[0].required_evidence = vec![RequiredEvidenceKind::Tests];
    });
    assert_rejected("status", |plan, _| {
        plan.steps[0].status = PlanStepStatus::Complete;
    });
    assert_rejected("extra-step", |plan, _| {
        plan.steps.push(archon_session::plan::PlanStep {
            number: 6,
            description: "missing durable task".into(),
            affected_files: vec![],
            status: PlanStepStatus::Pending,
            blocked_by: vec![],
            required_evidence: vec![],
            task_id: Some("missing-durable-task".into()),
        });
    });
    assert_rejected("duplicate-step", |plan, _| {
        plan.steps[1].task_id = plan.steps[0].task_id.clone();
    });
}

#[test]
fn failed_rehydration_installation_leaves_no_partial_tasks_or_attachment() {
    let (_db, store) = test_store();
    let source = TaskManager::new();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &source,
        &store,
        &test_plan_approval_authority(&store, "rehydration-atomic"),
        "rehydration-atomic",
        &mut plan,
    )
    .unwrap();
    let durable = store.load_plan_tasks("rehydration-atomic").unwrap();
    let restarted = TaskManager::new();

    restarted.fail_next_plan_task_installation_for_test("rehydration-atomic");
    let error = rehydrate_plan_tasks(
        &restarted,
        &store,
        &test_plan_approval_authority(&store, "rehydration-atomic"),
        "rehydration-atomic",
    )
    .unwrap_err();
    assert!(error.contains("injected plan task installation preparation failure"));
    assert_eq!(
        restarted.installed_plan_task_count_for_session_for_test("rehydration-atomic"),
        0
    );
    assert!(ids.iter().all(|id| restarted.get_task(id).is_none()));

    let error = restarted
        .insert_plan_task(test_task_info_from_persisted(
            "rehydration-atomic",
            &durable[0],
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        crate::task_manager::TaskTransitionError::Persistence(_)
    ));
}

#[test]
fn durable_transition_failure_leaves_memory_unchanged() {
    let manager = TaskManager::new();
    let mut plan = five_step_plan();
    let infos = build_plan_task_infos("missing-store", &mut plan).unwrap();
    manager.restore_plan_task(infos[0].clone()).unwrap();

    let error = manager
        .set_status_checked_with_evidence_ids(&infos[0].id, TaskStatus::Running, "", &[])
        .unwrap_err();
    assert!(matches!(
        error,
        crate::task_manager::TaskTransitionError::Persistence(_)
    ));
    assert_eq!(
        manager.get_task(&infos[0].id).unwrap().status,
        TaskStatus::Pending
    );
}

#[test]
fn plan_task_stop_persists_before_cancellation() {
    let manager = TaskManager::new();
    let mut plan = five_step_plan();
    let infos = build_plan_task_infos("missing-store", &mut plan).unwrap();
    manager.restore_plan_task(infos[0].clone()).unwrap();

    assert!(manager.stop_task(&infos[0].id).is_err());
    assert!(!manager.is_cancelled(&infos[0].id));
    assert_eq!(
        manager.get_task(&infos[0].id).unwrap().status,
        TaskStatus::Pending
    );
}

#[test]
fn manual_tasks_remain_process_scoped() {
    let manager = TaskManager::new();
    let id = manager.create_task("manual");
    assert!(manager.get_task(&id).unwrap().metadata.is_none());
    manager
        .set_status_checked_with_evidence_ids(&id, TaskStatus::Running, "", &[])
        .unwrap();
    assert_eq!(manager.get_task(&id).unwrap().status, TaskStatus::Running);
}

#[test]
fn persisted_plan_tasks_rehydrate_after_store_reopen() {
    let manager = TaskManager::new();
    let (_db, store) = test_store();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "reopen"),
        "reopen",
        &mut plan,
    )
    .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&ids[0], TaskStatus::Running, "", &[])
        .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&ids[0], TaskStatus::Completed, "", &[])
        .unwrap();
    let stored = store.load_plan_tasks("reopen").unwrap();
    assert_eq!(
        stored
            .iter()
            .find(|task| task.task_id == ids[0])
            .unwrap()
            .status,
        "Completed"
    );
    let persisted_plan = store.load_plan("reopen", &plan.id).unwrap().unwrap();
    assert_eq!(persisted_plan.steps[0].status, PlanStepStatus::Complete);
    let rehydrated = TaskManager::new();
    assert_eq!(
        rehydrate_plan_tasks(
            &rehydrated,
            &store,
            &test_plan_approval_authority(&store, "reopen"),
            "reopen"
        )
        .unwrap(),
        5
    );
    assert_eq!(
        rehydrate_plan_tasks(
            &rehydrated,
            &store,
            &test_plan_approval_authority(&store, "reopen"),
            "reopen"
        )
        .unwrap(),
        0
    );
    assert_eq!(
        rehydrated.get_task(&ids[0]).unwrap().status,
        TaskStatus::Completed
    );
}
