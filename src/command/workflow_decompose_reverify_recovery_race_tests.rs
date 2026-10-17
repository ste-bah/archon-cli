use super::*;

#[tokio::test]
async fn accepted_report_recovery_does_not_overwrite_concurrent_cancel() {
    let fixture = resume_fixture().await;
    let report_id = persist_accepted_reverify(&fixture);
    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap();
    let observed = fixture.store.load_state(&fixture.run_id).unwrap();
    let store = fixture.store.clone();
    let run_id = fixture.run_id.clone();
    crate::command::workflow::workflow_decompose_reverify::eligibility::set_before_recovery_lock_hook(move || {
        archon_workflow::LifecycleController::new(store)
            .apply(&run_id, archon_workflow::LifecycleAction::Cancel)
            .unwrap();
    });
    let result = crate::command::workflow::workflow_decompose_reverify::eligibility::recover_accepted_report_status(
        &fixture.store,
        &fixture.run_id,
        true,
        observed.generation,
        &observed.status,
        latest.as_ref(),
    )
    .unwrap();
    assert!(
        result.is_none(),
        "recovery unexpectedly overwrote cancel for {report_id}"
    );
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Cancelled
    );
}

#[tokio::test]
async fn resume_recovery_does_not_overwrite_concurrent_cancel() {
    let fixture = resume_fixture().await;
    persist_accepted_reverify(&fixture);
    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap();
    let store = fixture.store.clone();
    let run_id = fixture.run_id.clone();
    crate::command::workflow::workflow_decompose_reverify::eligibility::set_before_recovery_lock_hook(move || {
        archon_workflow::LifecycleController::new(store)
            .apply(&run_id, archon_workflow::LifecycleAction::Cancel)
            .unwrap();
    });
    let lease = crate::command::workflow_executor_lease::acquire(
        &fixture.store.run_dir(&fixture.run_id),
        &fixture.run_id,
    )
    .unwrap();
    let recovered = crate::command::workflow::workflow_decompose_reverify::eligibility::recover_accepted_reverify_before_resume(
        &fixture.store,
        &fixture.run_id,
        &lease,
        latest.as_ref(),
    )
    .unwrap();
    assert!(recovered.is_none());
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Cancelled
    );
}
