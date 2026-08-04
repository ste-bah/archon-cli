use super::*;

fn failed_review_wave_fixture() -> serde_json::Value {
    serde_json::from_str(archon_test_support::fixtures::WF66_REVIEW_REMEDIATION_WAVE_FAILED_MINIMAL)
        .expect("fixture")
}

#[test]
fn failed_review_remediation_wave_records_blocker_before_verification() {
    let review_fixes = failed_review_wave_fixture();
    let mut evidence = LifecycleEvidence::default();
    let block = review_remediation::review_remediation_block(
        &mut evidence,
        1,
        &serde_json::json!({ "tasks": [] }),
        &serde_json::json!({ "status": "needs_review" }),
        &serde_json::json!({ "items": [] }),
        &review_fixes,
    )
    .expect("review remediation should block");

    assert_eq!(block.id, "blocked-review-remediation-failed-1");
    assert_eq!(
        block.inputs["reviewRemediationFailures"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        evidence.repair_attempts[0]["issue_kind"],
        "review_remediation_unresolved"
    );
}

#[test]
fn accepted_review_remediation_wave_does_not_block() {
    let mut review_fixes = failed_review_wave_fixture();
    review_fixes["status"] = serde_json::json!("accepted");
    review_fixes["data"]["items"][0]["status"] = serde_json::json!("accepted");
    let mut evidence = LifecycleEvidence::default();

    let block = review_remediation::review_remediation_block(
        &mut evidence,
        1,
        &serde_json::json!({ "tasks": [] }),
        &serde_json::json!({ "status": "accepted" }),
        &serde_json::json!({ "items": [] }),
        &review_fixes,
    );

    assert!(block.is_none());
    assert!(evidence.repair_attempts.is_empty());
}
