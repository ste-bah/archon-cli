use super::*;

#[tokio::test]
async fn evidence_378_supplementary_only_subset_is_judged_and_merged() {
    let (temp, tasks, prd, mut contract, judge) = frozen().await;
    let reused = contract.acceptance[0].judgment.clone();
    supplementary_only_candidate(&mut contract);
    let before = format!(
        "judge_batches={:?}, AC-X-001={:?}, SUP-REQ-X-001={:?}",
        *judge.batches.lock().unwrap(),
        contract.acceptance[0].judgment.verdict,
        contract.supplementary[0].judgment.verdict
    );

    let prepared = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&contract).unwrap(),
        judge.clone(),
    )
    .await
    .expect("the merged contract passes full acceptance validation");

    assert_eq!(
        *judge.batches.lock().unwrap(),
        vec![vec!["SUP-REQ-X-001".to_string()]],
        "the accepted AC verdict is reused and only the new supplementary check is judged"
    );
    let merged: AcceptanceContract = serde_json::from_slice(&prepared.contract_bytes).unwrap();
    let after = format!(
        "judge_batches={:?}, AC-X-001={:?}, SUP-REQ-X-001={:?}",
        *judge.batches.lock().unwrap(),
        merged.acceptance[0].judgment.verdict,
        merged.supplementary[0].judgment.verdict
    );
    println!("EVIDENCE #378 supplementary-only BEFORE: {before} AFTER: {after}");
    assert_eq!(merged.acceptance[0].judgment, reused);
    assert_eq!(
        merged.supplementary[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}

/// #552 replaces `evidence_378_missing_supplementary_decision_remains_refused`:
/// the check whose decision the judge keeps omitting is still never accepted,
/// now as its author's finding (naming the omission) instead of a failed
/// freeze. The batch and missing-id assertions are kept.
#[tokio::test]
async fn evidence_378_missing_supplementary_decision_remains_refused_552() {
    let (temp, tasks, prd, mut contract, _) = frozen().await;
    supplementary_only_candidate(&mut contract);
    let judge = Arc::new(RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: true,
    });

    let before = format!(
        "candidate_acceptance_ids={:?}, candidate_supplementary_ids={:?}",
        contract
            .acceptance
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        contract
            .supplementary
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>()
    );
    let prepared = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&contract).unwrap(),
        judge.clone(),
    )
    .await
    .expect("a reply omitting the supplementary judgment goes back to its author");
    let after = format!(
        "findings={:?}",
        (prepared.findings.iter())
            .map(|f| &f.text)
            .collect::<Vec<_>>()
    );
    println!("EVIDENCE #378 missing-supplementary-decision BEFORE: {before} AFTER: {after}");

    assert!(prepared.non_accepted.contains("SUP-REQ-X-001"), "{after}");
    assert!(after.contains("missing=[\\\"SUP-REQ-X-001\\\"]"), "{after}");
    // #385: the same rejection class on every reply is no progress, so the
    // judge is asked once and then re-asked for the whole no-progress window
    // before the batch ends incomplete; each ask names only the new check.
    assert_eq!(
        judge.batches.lock().unwrap().as_slice(),
        // 1 + JUDGE_NO_PROGRESS_WINDOW (3) in workflow_task_set_judge.rs.
        vec![vec!["SUP-REQ-X-001".to_string()]; 4]
    );
}

#[test]
fn evidence_378_full_contract_without_acceptance_checks_is_rejected() {
    let mut empty = cache_contract();
    empty.acceptance.clear();
    let before = format!(
        "acceptance_count={}, supplementary_count={}",
        empty.acceptance.len(),
        empty.supplementary.len()
    );
    let expected: BTreeSet<String> = BTreeSet::new();
    let error =
        archon_workflow::task_set_contract::validate_acceptance_structure(&empty, &expected, true)
            .expect_err("full contract validation must still require an acceptance check");
    let after = format!("validation_error={error}");
    println!("EVIDENCE #378 full-contract-empty-acceptance BEFORE: {before} AFTER: {after}");
    assert!(after.contains("examined zero acceptance checks"));
}
