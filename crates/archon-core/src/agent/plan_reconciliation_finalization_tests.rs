use super::*;

#[test]
fn observation_failure_persistence_failure_blocks_completion_until_retry() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = format!("observation-failure-{}", uuid::Uuid::new_v4());
    let store = archon_session::plan::PlanStore::new(
        archon_session::storage::SessionStore::open(&temp.path().join("session.db"))
            .unwrap()
            .db(),
    )
    .unwrap();
    let mut plan = super::tests::approved_plan();
    plan.session_id = Some(session_id.clone());
    store.save_plan(&session_id, &plan).unwrap();
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = session_id;
    agent.plan_store = Some(store.clone());
    store.fail_next_observation_failure_persistence();

    assert!(
        agent
            .record_plan_observation_failure("snapshot failed")
            .is_err()
    );
    assert!(agent.plan_completion_block().is_some());
    assert!(agent.observation_failure_blocker.is_none());
    assert!(
        store
            .load_latest_plan(&agent.config.session_id)
            .unwrap()
            .unwrap()
            .execution_evidence
            .observation_failure
            .is_some()
    );
}

#[test]
fn active_plan_does_not_gate_ordinary_output() {
    let mut agent = active_plan_agent("ordinary-output");

    for output in [
        concat!("Not all tasks ", "complete."),
        "The work is not done; step remains incomplete.",
        "work is complete",
    ] {
        assert!(matches!(
            agent.finalization_verdict(output),
            TurnFinalizationVerdict::Allowed
        ));
    }
}

#[test]
fn explicit_finalization_is_blocked_by_durable_reconciliation() {
    let mut agent = active_plan_agent("explicit-finalization");
    agent.set_turn_finalization_callback(std::sync::Arc::new(|_, _| {
        TurnFinalizationVerdict::Allowed
    }));

    assert!(matches!(
        agent.finalization_verdict("ordinary response"),
        TurnFinalizationVerdict::Blocked { .. }
    ));
}

fn active_plan_agent(session_suffix: &str) -> Agent {
    let temp = tempfile::tempdir().unwrap();
    let session_id = format!("active-plan-{session_suffix}-{}", uuid::Uuid::new_v4());
    let store = archon_session::plan::PlanStore::new(
        archon_session::storage::SessionStore::open(&temp.path().join("session.db"))
            .unwrap()
            .db(),
    )
    .unwrap();
    let mut plan = super::tests::approved_plan();
    plan.session_id = Some(session_id.clone());
    store.save_plan(&session_id, &plan).unwrap();
    store
        .record_plan_observation_failure(&session_id, &plan.id, "fixture failure")
        .unwrap();
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = session_id;
    agent.plan_store = Some(store);
    agent
}

#[test]
fn actual_finalization_blocks_when_reconciliation_requires_repair() {
    let mut agent = super::super::tests::test_agent();
    let temp = tempfile::tempdir().unwrap();
    let session_id = format!("finalization-reconciliation-{}", uuid::Uuid::new_v4());
    let store = archon_session::plan::PlanStore::new(
        archon_session::storage::SessionStore::open(&temp.path().join("session.db"))
            .unwrap()
            .db(),
    )
    .unwrap();
    let mut plan = super::tests::approved_plan();
    plan.session_id = Some(session_id.clone());
    store.save_plan(&session_id, &plan).unwrap();
    agent.config.session_id = session_id.clone();
    agent.plan_store = Some(store.clone());
    store
        .record_plan_observation_failure(&session_id, &plan.id, "fixture failure")
        .unwrap();

    assert!(agent.plan_completion_block().is_some());
}
