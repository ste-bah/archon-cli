use super::*;

#[test]
fn completed_state_after_consumed_resume_is_eligible_for_reverify() {
    let fixture = fixture();
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    std::fs::create_dir_all(run_dir.join("decomposition/reverify")).unwrap();
    std::fs::write(
        run_dir.join("decomposition/reverify/consumed.jsonl"),
        "{\"report_id\":\"reverify-old-report\",\"subjects\":[\"UNKNOWN-X\"]}\n",
    )
    .unwrap();
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Completed;
    fixture.store.save_state(&run).unwrap();
    let lease =
        crate::command::workflow_executor_lease::acquire(&run_dir, &fixture.run_id).unwrap();
    let v2 = WorkflowV2ResultStore::new(run_dir.join("v2"));

    assert!(
        !crate::command::workflow::workflow_decompose_reverify::eligibility::check_run_status(
            &fixture.store,
            &run_dir,
            &fixture.run_id,
            &lease,
            RunStatus::Completed,
            None,
            &v2,
        )
        .unwrap()
    );
}

#[tokio::test]
async fn consumed_paused_ledger_is_refused_for_reverify_and_marked_by_shared_helper() {
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], b"not a task body\n").unwrap();
    let first = invoke_reverify(&archon_binary(), &fixture);
    assert!(first.status.success());
    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Paused
    );
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let ledger = crate::command::workflow_decompose_remediation::read(&run_dir, &latest.id)
        .unwrap()
        .unwrap();
    crate::command::workflow_decompose_remediation::append_consumed(&run_dir, &ledger).unwrap();
    assert!(
        crate::command::workflow_decompose_remediation::is_consumed(&run_dir, &latest.id).unwrap()
    );

    let output = invoke_reverify(&archon_binary(), &fixture);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("remediation ledger is already consumed"),
        "{stderr}"
    );
}
