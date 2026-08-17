use super::*;
use crate::plan_tasks::test_plan_approval_authority;
use std::sync::{Arc, Barrier};

fn approved_plan(plan_id: &str) -> PlanDocument {
    let mut plan = PlanDocument::new(plan_id, "Concurrent materialization plan");
    plan.status = PlanStatus::Approved;
    plan.approval = Some(archon_session::plan::PlanApproval {
        decision: archon_session::plan::PlanApprovalDecision::Approve,
        source: archon_session::plan::PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    });
    plan.steps = (1..=2)
        .map(|number| archon_session::plan::PlanStep {
            number,
            description: format!("step {number}"),
            affected_files: vec![],
            status: PlanStepStatus::Pending,
            blocked_by: if number == 1 { vec![] } else { vec![1] },
            required_evidence: vec![],
            task_id: None,
        })
        .collect();
    plan
}

fn test_store() -> PlanStore {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    PlanStore::new(&db).unwrap()
}

#[test]
fn repeated_materialization_uses_existing_canonical_generation() {
    let store = test_store();
    let manager = TaskManager::new();
    let session_id = "same-plan-sequential";
    let mut first = approved_plan("same-approved-plan");
    let first_ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, session_id),
        session_id,
        &mut first,
    )
    .unwrap();
    let mut retry = approved_plan("same-approved-plan");

    let retry_ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, session_id),
        session_id,
        &mut retry,
    )
    .unwrap();

    assert_eq!(retry_ids, first_ids);
    assert_eq!(retry.to_json(), first.to_json());
    assert_eq!(
        store.load_plan_tasks(session_id).unwrap().len(),
        first_ids.len()
    );
    assert_eq!(
        manager.installed_plan_task_count_for_session_for_test(session_id),
        first_ids.len()
    );
}

#[test]
fn different_plans_in_one_session_claim_independent_generations() {
    let store = test_store();
    let manager = TaskManager::new();
    let session_id = "different-plans-one-session";
    let mut first = approved_plan("first-approved-plan");
    let mut second = approved_plan("second-approved-plan");

    let first_ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, session_id),
        session_id,
        &mut first,
    )
    .unwrap();
    let second_ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, session_id),
        session_id,
        &mut second,
    )
    .unwrap();

    assert!(first_ids.iter().all(|id| !second_ids.contains(id)));
    assert_eq!(store.load_plan_tasks(session_id).unwrap().len(), 4);
    let restarted = TaskManager::new();
    assert_eq!(
        rehydrate_plan_tasks(
            &restarted,
            &store,
            &test_plan_approval_authority(&store, session_id),
            session_id
        )
        .unwrap(),
        4
    );
}

#[test]
fn legacy_generation_is_adopted_without_rewrite_and_rehydrates() {
    let store = test_store();
    let session_id = "legacy-generation-adoption";
    let mut plan = approved_plan("legacy-approved-plan");
    let infos = build_plan_task_infos(session_id, &mut plan).unwrap();
    let tasks = persisted_records(&infos).unwrap();
    let record = archon_session::plan::PlanApprovalRecord {
        plan_id: plan.id.clone(),
        session_id: session_id.into(),
        approval: plan.approval.clone().expect("approved plan approval"),
    };
    let authority = test_plan_approval_authority(&store, session_id);
    store
        .save_terminal_plan_with_approval(&authority, session_id, &plan, &record)
        .unwrap();

    for task in &tasks {
        store.save_plan_task_fixture(session_id, task).unwrap();
    }
    store
        .save_plan_with_tasks(&authority, session_id, &plan, &tasks)
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
        tasks.len()
    );
    let mut replacement = plan.clone();
    replacement.steps[0].task_id = Some("replacement-task".into());
    let replacement_task = PersistedPlanTask {
        task_id: "replacement-task".into(),
        ..tasks[0].clone()
    };
    let dependent_task = PersistedPlanTask {
        blocked_by: vec![replacement_task.task_id.clone()],
        ..tasks[1].clone()
    };
    let error = store
        .save_plan_with_tasks(
            &authority,
            session_id,
            &replacement,
            &[replacement_task, dependent_task],
        )
        .expect_err("adopted legacy generation must remain canonical");
    assert!(error.to_string().contains("materialization"));
}

#[test]
fn concurrent_same_plan_materialization_has_one_canonical_durable_generation() {
    let store = test_store();
    let session_id = "same-plan-concurrent";
    let barrier = Arc::new(Barrier::new(3));
    let _reset =
        set_materialization_barrier_for_test(session_id.into(), Some(Arc::clone(&barrier)));
    let first_manager = Arc::new(TaskManager::new());
    let second_manager = Arc::new(TaskManager::new());
    let first_store = store.clone();
    let second_store = store.clone();
    let first_manager_for_thread = Arc::clone(&first_manager);
    let second_manager_for_thread = Arc::clone(&second_manager);

    let first = std::thread::spawn(move || {
        let mut plan = approved_plan("shared-approved-plan");
        let result = materialize_plan_tasks(
            &first_manager_for_thread,
            &first_store,
            &test_plan_approval_authority(&first_store, session_id),
            session_id,
            &mut plan,
        );
        (result, plan)
    });
    let second = std::thread::spawn(move || {
        let mut plan = approved_plan("shared-approved-plan");
        let result = materialize_plan_tasks(
            &second_manager_for_thread,
            &second_store,
            &test_plan_approval_authority(&second_store, session_id),
            session_id,
            &mut plan,
        );
        (result, plan)
    });
    barrier.wait();
    let (first_result, first_plan) = first.join().unwrap();
    let (second_result, second_plan) = second.join().unwrap();

    assert_eq!(
        usize::from(first_result.is_ok()) + usize::from(second_result.is_ok()),
        1
    );
    let winner_ids = first_result.as_ref().or(second_result.as_ref()).unwrap();
    let loser_plan = if first_result.is_err() {
        &first_plan
    } else {
        &second_plan
    };
    assert!(loser_plan.steps.iter().all(|step| step.task_id.is_none()));

    let canonical = store
        .load_plan(session_id, "shared-approved-plan")
        .unwrap()
        .unwrap();
    let durable = store.load_plan_tasks(session_id).unwrap();
    assert_eq!(durable.len(), winner_ids.len());
    assert_eq!(
        canonical
            .steps
            .iter()
            .map(|step| step.task_id.as_ref().unwrap())
            .collect::<Vec<_>>(),
        winner_ids.iter().collect::<Vec<_>>()
    );
    let durable_ids = durable
        .iter()
        .map(|task| task.task_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let winner_ids = winner_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(durable_ids, winner_ids);
    let restarted = TaskManager::new();
    assert_eq!(
        rehydrate_plan_tasks(
            &restarted,
            &store,
            &test_plan_approval_authority(&store, session_id),
            session_id
        )
        .unwrap(),
        winner_ids.len()
    );
}
