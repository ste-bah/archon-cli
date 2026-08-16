use super::test_support::{approve_plan, five_step_plan, test_store};
use super::*;
use crate::plan_tasks::test_plan_approval_authority;
use serde_json::json;

#[test]
fn approved_five_step_plan_creates_five_linked_tasks() {
    let manager = TaskManager::new();
    let store = test_store();
    let mut plan = five_step_plan();
    let task_ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "session-five"),
        "session-five",
        &mut plan,
    )
    .unwrap();
    assert_eq!(task_ids.len(), 5);
    for (index, step) in plan.steps.iter().enumerate() {
        let metadata = manager
            .get_task(step.task_id.as_ref().unwrap())
            .unwrap()
            .metadata
            .unwrap();
        assert_eq!(metadata.plan_id, plan.id);
        assert_eq!(metadata.plan_step, step.number);
        assert_eq!(
            metadata.blocked_by,
            task_ids[..index]
                .last()
                .cloned()
                .into_iter()
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(store.load_plan_tasks("session-five").unwrap().len(), 5);
    assert_eq!(plan.steps.len(), 5);
}

#[test]
fn independent_task_can_start_after_another_task_fails_or_stops() {
    for terminal_status in [TaskStatus::Failed, TaskStatus::Stopped] {
        let manager = TaskManager::new();
        let store = test_store();
        let mut plan = PlanDocument::new("independent-terminal-plan", "Independent terminal tasks");
        plan.status = PlanStatus::Approved;
        plan.steps = (1..=2)
            .map(|number| archon_session::plan::PlanStep {
                number,
                description: format!("step {number}"),
                affected_files: vec![],
                status: PlanStepStatus::Pending,
                blocked_by: vec![],
                required_evidence: vec![],
                task_id: None,
            })
            .collect();
        let ids = {
            approve_plan(&store, "independent-terminal", &mut plan);
            materialize_plan_tasks(
                &manager,
                &store,
                &test_plan_approval_authority(&store, "independent-terminal"),
                "independent-terminal",
                &mut plan,
            )
        }
        .expect("materialize approved plan");

        manager
            .set_status_checked_with_evidence_ids(&ids[0], terminal_status, "", &[])
            .expect("terminal transition persists");
        manager
            .set_status_checked_with_evidence_ids(&ids[1], TaskStatus::Running, "", &[])
            .expect("independent pending task remains transitionable");
    }
}

#[test]
fn blocked_step_cannot_start_before_dependency_completes() {
    let manager = TaskManager::new();
    let store = test_store();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "blocked"),
        "blocked",
        &mut plan,
    )
    .unwrap();
    assert!(matches!(
        manager.set_status_checked_with_evidence_ids(&ids[1], TaskStatus::Running, "", &[]),
        Err(crate::task_manager::TaskTransitionError::BlockedDependency { .. })
    ));
    manager
        .set_status_checked_with_evidence_ids(&ids[0], TaskStatus::Running, "", &[])
        .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&ids[0], TaskStatus::Completed, "", &[])
        .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&ids[1], TaskStatus::Running, "", &[])
        .unwrap();
}

#[test]
fn tests_required_step_cannot_complete_without_passed_tests() {
    let manager = TaskManager::new();
    let store = test_store();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "evidence"),
        "evidence",
        &mut plan,
    )
    .unwrap();
    for id in &ids[..2] {
        manager
            .set_status_checked_with_evidence_ids(id, TaskStatus::Running, "", &[])
            .unwrap();
        manager
            .set_status_checked_with_evidence_ids(id, TaskStatus::Completed, "", &[])
            .unwrap();
    }
    manager
        .set_status_checked_with_evidence_ids(&ids[2], TaskStatus::Running, "", &[])
        .unwrap();
    assert!(matches!(
        manager.set_status_checked_with_evidence_ids(&ids[2], TaskStatus::Completed, "", &[]),
        Err(crate::task_manager::TaskTransitionError::MissingEvidence { .. })
    ));
    let forged = manager.set_status_checked_with_evidence_ids(
        &ids[2],
        TaskStatus::Completed,
        "model-run",
        &["model-asserted-passed".to_string()],
    );
    assert!(
        matches!(
            forged,
            Err(crate::task_manager::TaskTransitionError::UntrustedEvidence(
                _
            ))
        ),
        "forged evidence must remain untrusted: {forged:?}"
    );
}

#[test]
fn materialization_rejects_duplicate_pending_step_ids_before_persistence() {
    let manager = TaskManager::new();
    let store = test_store();
    let mut plan = five_step_plan();
    plan.steps[0].task_id = Some("duplicated-plan-task-id".into());
    plan.steps[1].task_id = Some("duplicated-plan-task-id".into());

    let error = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "duplicate-pending"),
        "duplicate-pending",
        &mut plan,
    )
    .expect_err("duplicate supplied plan step IDs must fail before persistence");
    assert!(error.contains("pending batch"));
    assert!(
        store
            .load_plan_tasks("duplicate-pending")
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .load_plan("duplicate-pending", &plan.id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        manager.installed_plan_task_count_for_session_for_test("duplicate-pending"),
        0
    );
}

#[test]
fn materialization_rejects_existing_durable_task_id_without_overwrite() {
    let manager = TaskManager::new();
    let store = test_store();
    let durable = PersistedPlanTask {
        task_id: "durable-task-id".into(),
        plan_id: "existing-plan".into(),
        plan_step: 1,
        description: "durable task".into(),
        status: "Running".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    store
        .save_plan_task_fixture("durable-collision", &durable)
        .expect("seed durable task");
    let mut plan = five_step_plan();
    plan.steps[0].task_id = Some(durable.task_id.clone());

    let error = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "durable-collision"),
        "durable-collision",
        &mut plan,
    )
    .expect_err("a durable task ID collision must fail before persistence");
    assert!(error.contains("durable plan task"));
    let stored = store.load_plan_tasks("durable-collision").unwrap();
    assert_eq!(stored, vec![durable]);
    assert!(
        store
            .load_plan("durable-collision", &plan.id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        manager.installed_plan_task_count_for_session_for_test("durable-collision"),
        0
    );
}

#[test]
fn materialization_rejects_existing_manager_task_id_without_durable_overwrite() {
    let manager = TaskManager::new();
    let store = test_store();
    let colliding_id = manager.create_task("unrelated manual task");
    let mut plan = five_step_plan();
    plan.steps[0].task_id = Some(colliding_id.clone());

    let error = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "manager-collision"),
        "manager-collision",
        &mut plan,
    )
    .expect_err("an existing manual task must reject the plan");
    assert!(error.contains("task ID collision"));
    assert_eq!(
        manager.get_task(&colliding_id).unwrap().description,
        "unrelated manual task"
    );
    assert!(
        store
            .load_plan_tasks("manager-collision")
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .load_plan("manager-collision", &plan.id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn task_list_exposes_plan_progress_and_blocked_by() {
    let manager = TaskManager::new();
    let store = test_store();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "list"),
        "list",
        &mut plan,
    )
    .unwrap();
    let tasks = task_list_json(&manager);
    let second = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["id"] == ids[1])
        .unwrap();
    assert_eq!(second["plan_id"], "plan-five");
    assert_eq!(second["plan_step"], 2);
    assert_eq!(second["blocked_by"], json!([ids[0]]));
    assert_eq!(second["plan_progress"], json!({"completed": 0, "total": 5}));
}

#[test]
fn two_sessions_persist_plan_tasks_to_their_own_stores() {
    let manager = TaskManager::new();
    let first_store = test_store();
    let second_store = test_store();
    let mut first_plan = five_step_plan();
    let mut second_plan = five_step_plan();
    second_plan.id = "plan-second".to_string();
    let first_ids = materialize_plan_tasks(
        &manager,
        &first_store,
        &test_plan_approval_authority(&first_store, "first"),
        "first",
        &mut first_plan,
    )
    .unwrap();
    let second_ids = materialize_plan_tasks(
        &manager,
        &second_store,
        &test_plan_approval_authority(&second_store, "second"),
        "second",
        &mut second_plan,
    )
    .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&first_ids[0], TaskStatus::Running, "", &[])
        .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&second_ids[0], TaskStatus::Running, "", &[])
        .unwrap();
    assert_eq!(
        first_store
            .load_plan_tasks("first")
            .unwrap()
            .into_iter()
            .find(|task| task.task_id == first_ids[0])
            .unwrap()
            .status,
        "Running"
    );
    assert_eq!(
        second_store
            .load_plan_tasks("second")
            .unwrap()
            .into_iter()
            .find(|task| task.task_id == second_ids[0])
            .unwrap()
            .status,
        "Running"
    );
}
