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
