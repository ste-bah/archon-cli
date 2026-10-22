use super::*;

#[tokio::test]
async fn evidence_385_missing_decision_is_reasked_with_a_fresh_prompt() {
    let rejected = r#"{"decisions":[]}"#;
    let client = Scripted::new(vec![
        Ok((rejected, Some("end_turn"))),
        Ok((ACCEPTED, Some("end_turn"))),
    ]);
    judge_contract(&client, contract(), &expected())
        .await
        .expect("the corrected judgment is accepted");
    let calls = client.calls();
    let retry = &calls[1];
    assert_eq!(calls.len(), 2);
    assert_eq!(retry.len(), 1);
    assert!(retry[0]["content"].as_str().unwrap().contains("AC-X-001"));
}

#[tokio::test]
async fn evidence_385_malformed_reply_carries_its_exact_parse_error_into_retry() {
    let rejected = "not-json";
    let client = Scripted::new(vec![
        Ok((rejected, Some("end_turn"))),
        Ok((ACCEPTED, Some("end_turn"))),
    ]);
    let before = format!(
        "calls={}, rejected_reply={rejected:?}",
        client.calls().len()
    );
    judge_contract(&client, contract(), &expected())
        .await
        .expect("the corrected judgment is accepted");
    let calls = client.calls();
    let retry = &calls[1];
    let after = format!(
        "calls={}, assistant_reply={:?}, retry_error={:?}",
        calls.len(),
        retry[1]["content"],
        retry[2]["content"]
    );
    println!("EVIDENCE #385 judge-malformed-json BEFORE: {before} AFTER: {after}");
    assert_eq!(retry[1]["content"], rejected);
    assert_eq!(
        retry[2]["content"],
        "Your reply was rejected: acceptance judge returned malformed batched JSON (expected ident at line 1 column 2; at the fault: n<HERE>ot-json); retry the full batch: expected ident at line 1 column 2\nReply again with the complete JSON document required by the first message."
    );
}

#[tokio::test]
async fn evidence_385_output_limited_reply_is_reasked_as_a_fresh_judgment() {
    let client = Scripted::new(vec![
        Ok((ACCEPTED, Some("max_tokens"))),
        Ok((REFUTED, Some("end_turn"))),
    ]);
    let before = format!("calls={}, stop_reason=max_tokens", client.calls().len());
    let judged = judge_contract(&client, contract(), &expected())
        .await
        .expect("the complete fresh judgment is accepted as a reply");
    let calls = client.calls();
    let after = format!(
        "calls={}, retry_messages={}, verdict={:?}",
        calls.len(),
        calls[1].len(),
        judged.acceptance[0].judgment.verdict
    );
    println!("EVIDENCE #385 judge-output-limit BEFORE: {before} AFTER: {after}");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].len(), calls[0].len());
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
}

#[test]
fn an_incremental_subset_reports_a_missing_supplementary_id() {
    let mut subset = supplementary_subset();
    let missing = apply_judgments(&mut subset, r#"{"decisions":[]}"#).unwrap();

    assert_eq!(missing, BTreeSet::from(["SUP-1".to_string()]));
}

#[test]
fn a_full_contract_with_no_acceptance_checks_is_still_rejected() {
    let mut empty = contract();
    empty.acceptance.clear();
    let error = archon_workflow::task_set_contract::validate_acceptance_structure(
        &empty,
        &BTreeSet::new(),
        true,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("examined zero acceptance checks")
    );
}

#[tokio::test]
async fn a_mixed_incremental_subset_accepts_all_its_decisions() {
    let mut subset = contract();
    let mut supplementary = subset.acceptance[0].clone();
    supplementary.id = "SUP-1".into();
    subset.supplementary.push(supplementary);
    let client = Scripted::new(vec![Ok((
        r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"valid"},{"id":"SUP-1","verdict":"accepted","counterexample":"none","reason":"valid"}]}"#,
        Some("end_turn"),
    ))]);

    let judged = judge_contract_subset_resumable(&client, subset, None)
        .await
        .unwrap();

    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(
        judged.supplementary[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}
