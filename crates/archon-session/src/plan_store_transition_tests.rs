use super::*;

#[test]
fn checked_terminal_transition_rolls_back_task_step_and_reconciliation_together() {
    let (store, plan, task) = running_materialized_task();
    let session_id = "checked-transition-rollback";
    let running_plan = store
        .load_plan(session_id, &plan.id)
        .unwrap()
        .expect("running plan");
    let running_task = store.load_plan_tasks(session_id).unwrap().remove(0);
    store.fail_next_task_transition_after_plan_write();

    let authority = test_authority(&store, session_id);
    let error = store
        .transition_plan_task_checked(
            &authority,
            session_id,
            &task.task_id,
            "Running",
            "Completed",
            "",
            &[],
        )
        .expect_err("injected post-plan-write failure must roll back");

    assert!(
        error
            .to_string()
            .contains("injected task transition failure")
    );
    assert_materialized_plan_unchanged(&store, session_id, &running_plan, &running_task);
}

fn running_materialized_task() -> (PlanStore, PlanDocument, PersistedPlanTask) {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "checked-transition-rollback";
    let mut plan = PlanDocument::new("checked-transition-plan", "Checked transition rollback");
    plan.status = PlanStatus::Approved;
    plan.steps.push(PlanStep {
        number: 1,
        task_id: Some("checked-transition-task".into()),
        description: "first".into(),
        affected_files: Vec::new(),
        status: PlanStepStatus::Pending,
        blocked_by: vec![],
        required_evidence: vec![],
    });
    approve_for_materialization(&store, session_id, &mut plan);
    let task = PersistedPlanTask {
        task_id: "checked-transition-task".into(),
        plan_id: plan.id.clone(),
        plan_step: 1,
        description: "first".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        completion_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    let authority = test_authority(&store, session_id);
    store
        .save_plan_with_tasks(&authority, session_id, &plan, std::slice::from_ref(&task))
        .expect("seed");
    store
        .transition_plan_task_checked(
            &authority,
            session_id,
            &task.task_id,
            "Pending",
            "Running",
            "",
            &[],
        )
        .expect("start canonical task");
    (store, plan, task)
}
