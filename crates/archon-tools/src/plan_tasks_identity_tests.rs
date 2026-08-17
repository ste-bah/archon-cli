use super::*;
use crate::plan_tasks::test_plan_approval_authority;
use archon_session::storage::SessionStore;
use std::sync::{Arc, Barrier};

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
                vec![archon_completion::RequiredEvidenceKind::Tests]
            } else {
                vec![]
            },
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
fn separately_created_plan_stores_share_in_memory_database_identity() {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let first = PlanStore::new(&db).unwrap();
    let second = PlanStore::new(&db).unwrap();

    assert!(first.is_same_store(&second));
    assert!(first.is_same_store(&first.clone()));
}

#[test]
fn concurrent_plan_store_creation_over_shared_memory_database_is_atomic() {
    const WORKERS: usize = 32;
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let barrier = Arc::new(Barrier::new(WORKERS));
    let stores = std::thread::scope(|scope| {
        let handles = (0..WORKERS)
            .map(|_| {
                let db = db.clone();
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    PlanStore::new(&db).expect("concurrent store construction must succeed")
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("worker must not panic"))
            .collect::<Vec<_>>()
    });

    assert!(
        stores
            .iter()
            .skip(1)
            .all(|store| stores[0].is_same_store(store))
    );
}

#[test]
fn cloned_in_memory_database_preserves_plan_store_identity_for_rehydration() {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let first = PlanStore::new(&db).unwrap();
    let cloned_db = db.clone();
    let second = PlanStore::new(&cloned_db).unwrap();
    let session_id = "cloned-in-memory-plan-store";
    let source = TaskManager::new();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        &source,
        &first,
        &test_plan_approval_authority(&first, session_id),
        session_id,
        &mut plan,
    )
    .unwrap();
    let manager = TaskManager::new();

    manager
        .attach_plan_store(first.clone(), session_id)
        .unwrap();
    assert!(first.is_same_store(&second));
    assert_eq!(
        rehydrate_plan_tasks(
            &manager,
            &second,
            &test_plan_approval_authority(&second, session_id),
            session_id
        )
        .unwrap(),
        ids.len()
    );
    assert!(ids.iter().all(|id| manager.get_task(id).is_some()));
}

#[test]
fn independent_in_memory_databases_have_distinct_plan_store_identities() {
    let first = test_store();
    let second = test_store();

    assert!(!first.is_same_store(&second));
}

#[test]
fn durable_path_aliases_and_reopens_share_plan_store_identity() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("session.db");
    let relative = temp.path().join(".").join("session.db");
    let first_session_store = SessionStore::open(&database).unwrap();
    let second_session_store = SessionStore::open(&relative).unwrap();
    let first = PlanStore::new(first_session_store.db()).unwrap();
    let second = PlanStore::new(second_session_store.db()).unwrap();

    assert!(first.is_same_store(&second));

    #[cfg(unix)]
    {
        let alias = temp.path().join("session-alias.db");
        std::os::unix::fs::symlink(&database, &alias).unwrap();
        let alias_session_store = SessionStore::open(&alias).unwrap();
        let alias_store = PlanStore::new(alias_session_store.db()).unwrap();
        assert!(first.is_same_store(&alias_store));
    }
}

#[test]
fn distinct_durable_databases_have_distinct_plan_store_identities() {
    let temp = tempfile::tempdir().unwrap();
    let first_session_store = SessionStore::open(&temp.path().join("first.db")).unwrap();
    let second_session_store = SessionStore::open(&temp.path().join("second.db")).unwrap();
    let first = PlanStore::new(first_session_store.db()).unwrap();
    let second = PlanStore::new(second_session_store.db()).unwrap();

    assert!(!first.is_same_store(&second));
}
