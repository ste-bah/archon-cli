//! Every acceptance round covers the whole frozen contract.

use super::*;

/// Every round runs the WHOLE contract, whatever the script asks for, so a
/// check that passed earlier and regressed under a later fix cannot hide;
/// asking for a check outside the frozen contract is an operational error.
#[tokio::test]
async fn every_round_reruns_the_whole_contract_so_a_regression_cannot_hide() {
    let fixture = fixture(true);
    let first = run(&fixture, &execution(1, 3, &[])).await.unwrap();
    assert_eq!(failing_ids(&first), vec!["REQ-2", "REQ-9"]);
    // A remediation for REQ-2 breaks REQ-1, which passed in round 1.
    std::fs::remove_file(fixture.repo.path().join("present")).unwrap();
    let result = run(&fixture, &execution(2, 3, &["REQ-2"])).await.unwrap();
    let (record, _) = latest_round_record(&fixture.store.run_dir(&fixture.run_id))
        .unwrap()
        .unwrap();
    assert_eq!(record.round, 2);
    assert_eq!(record.requested_check_ids, vec!["REQ-2"]);
    assert_eq!(
        record
            .checks
            .iter()
            .map(|c| c.check_id.as_str())
            .collect::<Vec<_>>(),
        vec!["REQ-1", "REQ-2", "REQ-9"]
    );
    assert_eq!(failing_ids(&result), vec!["REQ-1", "REQ-2", "REQ-9"]);
    let bogus = run(&fixture, &execution(3, 3, &["REQ-404"])).await.unwrap();
    assert_eq!(bogus.status, WorkflowV2Status::NeedsReview);
    assert!(bogus.summary.contains("REQ-404"), "{}", bogus.summary);
}

/// Batch E: a failing check whose output points into a file another task
/// owns names that task as a writer on the round record and in the reply.
#[tokio::test]
async fn a_failure_in_another_tasks_file_names_that_task_as_a_writer() {
    // `grep` finds the line and fails on the missing path: its output names
    // the file with a line location the check itself never spells.
    let mut fixture = fixture_with(true, "grep -rn needle src missing-dir");
    let engine = fixture.repo.path().join("src/engine.rs");
    std::fs::create_dir_all(engine.parent().unwrap()).unwrap();
    std::fs::write(&engine, "fn a() {}\nfn b() {}\n// needle\n").unwrap();
    fixture.universe.tasks[0].files_expected_to_change = vec!["src/engine.rs".into()];
    let result = run(&fixture, &execution(1, 3, &[])).await.unwrap();
    let (record, _) = latest_round_record(&fixture.store.run_dir(&fixture.run_id))
        .unwrap()
        .unwrap();
    let check = record
        .checks
        .iter()
        .find(|c| c.check_id == "REQ-2")
        .unwrap();
    let routing = check
        .routing
        .as_ref()
        .unwrap_or_else(|| panic!("{check:#?}"));
    assert_eq!(routing.implicated_files, ["src/engine.rs"]);
    assert_eq!(routing.writer_tasks, ["TASK-F-001"]);
    let failing = result.data["failing"].as_array().unwrap();
    let reply = failing.iter().find(|f| f["check_id"] == "REQ-2").unwrap();
    assert_eq!(
        reply["routing"]["writer_tasks"],
        serde_json::json!(["TASK-F-001"])
    );
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|g| g.description.contains("routed also to TASK-F-001"))
    );
}
