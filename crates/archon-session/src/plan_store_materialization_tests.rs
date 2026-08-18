use std::sync::{Arc, Barrier};

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

#[test]
fn legacy_adoption_does_not_claim_rows_changed_after_validation() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "legacy-adoption-cas";
    let (canonical_plan, canonical_task) = legacy_generation("legacy-adoption-plan", "legacy-task");
    let (competing_plan, _competing_task) =
        legacy_generation("legacy-adoption-plan", "competing-legacy-task");
    seed_legacy_generation(&store, session_id, &canonical_plan, &canonical_task);

    let validated = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let _reset = PlanStore::set_legacy_adoption_barrier_for_test(
        Arc::clone(&validated),
        Arc::clone(&resume),
    );
    let adopter = store.clone();
    let adoption_plan = canonical_plan.clone();
    let adoption_task = canonical_task.clone();
    let adoption = std::thread::spawn(move || {
        adopter.save_plan_with_tasks(
            &test_authority(&adopter, session_id),
            session_id,
            &adoption_plan,
            &[adoption_task],
        )
    });
    validated.wait();
    let competing_store = store.clone();
    let competing_plan_for_thread = competing_plan.clone();
    let competing_plan_write = std::thread::spawn(move || {
        competing_store.save_plan(session_id, &competing_plan_for_thread)
    });
    let competing_store = store.clone();
    let competing_task = PersistedPlanTask {
        task_id: "competing-legacy-task".into(),
        plan_id: canonical_plan.id.clone(),
        plan_step: 1,
        description: "competing legacy task".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        completion_evidence: vec![],
        updated_at: "2026-08-15T00:00:01Z".into(),
    };
    let competing_task_write = std::thread::spawn(move || {
        competing_store.save_plan_task_fixture(session_id, &competing_task)
    });
    resume.wait();

    adoption
        .join()
        .expect("adoption thread")
        .expect("validated legacy generation must be claimed unchanged");
    for competing in [competing_plan_write, competing_task_write] {
        let competing = competing
            .join()
            .expect("competing mutation thread")
            .expect_err("a mutation racing a completed adoption must be rejected");
        assert!(
            competing
                .to_string()
                .contains("materialized canonical plan")
        );
    }
    assert!(
        materialization_claim_exists(&store, session_id, &canonical_plan.id),
        "adoption must claim the generation it validated"
    );
    assert_eq!(
        store
            .load_plan(session_id, &canonical_plan.id)
            .expect("load")
            .expect("plan")
            .to_json(),
        canonical_plan.to_json()
    );
    assert_eq!(
        store.load_plan_tasks(session_id).expect("tasks"),
        vec![canonical_task]
    );
}

#[test]
fn materialization_rejects_a_same_id_plan_that_differs_from_durable_approval() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "approval-bound-canonical-plan";
    let (canonical, canonical_task) = legacy_generation("approval-bound-plan", "canonical-task");
    let approval = canonical.approval.clone().expect("canonical approval");
    let record = PlanApprovalRecord {
        plan_id: canonical.id.clone(),
        session_id: session_id.into(),
        approval,
    };
    store
        .save_terminal_plan_with_approval(
            &test_authority(&store, session_id),
            session_id,
            &canonical,
            &record,
        )
        .expect("persist approved canonical plan");

    let (forged, forged_task) = legacy_generation("approval-bound-plan", "forged-task");
    let error = store
        .claim_plan_materialization_with_tasks(
            &test_authority(&store, session_id),
            session_id,
            &forged,
            &[forged_task],
        )
        .expect_err("a different same-ID plan must not inherit approval");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(
        store
            .load_plan(session_id, &canonical.id)
            .expect("load canonical plan")
            .expect("canonical plan")
            .to_json(),
        canonical.to_json()
    );
    assert!(
        store
            .load_plan_tasks(session_id)
            .expect("load tasks")
            .is_empty()
    );
    assert!(!materialization_claim_exists(
        &store,
        session_id,
        &canonical.id
    ));
    assert_eq!(canonical_task.plan_id, canonical.id);
}

#[test]
fn materialization_requires_a_matching_durable_approval_record() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "approval-required-materialization";
    let (plan, task) = legacy_generation("approval-required-plan", "approval-required-task");

    let error = store
        .claim_plan_materialization_with_tasks(
            &test_authority(&store, session_id),
            session_id,
            &plan,
            &[task],
        )
        .expect_err("approved plan without durable approval must not materialize");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(store.load_plan(session_id, &plan.id).unwrap().is_none());
    assert!(store.load_plan_tasks(session_id).unwrap().is_empty());
    assert!(!materialization_claim_exists(&store, session_id, &plan.id));
}
fn legacy_generation(plan_id: &str, task_id: &str) -> (PlanDocument, PersistedPlanTask) {
    let mut plan = PlanDocument::new(plan_id, "Legacy materialization");
    plan.status = PlanStatus::Approved;
    plan.approval = Some(PlanApproval {
        decision: PlanApprovalDecision::Approve,
        source: PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    });
    plan.steps.push(PlanStep {
        task_id: Some(task_id.into()),
        ..step(1, task_id)
    });
    let task = PersistedPlanTask {
        task_id: task_id.into(),
        plan_id: plan.id.clone(),
        plan_step: 1,
        description: task_id.into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        completion_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    (plan, task)
}

fn seed_legacy_generation(
    store: &PlanStore,
    session_id: &str,
    plan: &PlanDocument,
    task: &PersistedPlanTask,
) {
    let record = PlanApprovalRecord {
        plan_id: plan.id.clone(),
        session_id: session_id.into(),
        approval: plan.approval.clone().expect("legacy approval"),
    };
    store
        .save_terminal_plan_with_approval(
            &test_authority(store, session_id),
            session_id,
            plan,
            &record,
        )
        .expect("persist approved legacy plan");
    store
        .save_plan_task_fixture(session_id, task)
        .expect("legacy task");
}

fn materialization_claim_exists(store: &PlanStore, session_id: &str, plan_id: &str) -> bool {
    store
        .materialization_claim_exists_for_test(session_id, plan_id)
        .expect("read claim")
}

fn step(number: u32, description: &str) -> PlanStep {
    PlanStep {
        number,
        description: description.into(),
        affected_files: Vec::new(),
        status: PlanStepStatus::Pending,
        blocked_by: Vec::new(),
        required_evidence: Vec::new(),
        task_id: None,
    }
}
