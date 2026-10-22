use super::*;

#[tokio::test]
async fn acceptance_candidate_prepares_exact_staged_bytes_without_reading_or_writing_live_contract()
{
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, candidate) = seed(&temp);
    std::fs::write(
        tasks.join(ACCEPTANCE_CONTRACT_FILE),
        b"malformed live sentinel",
    )
    .unwrap();
    let response = r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted","reason":"rejects"}]}"#;

    let prepared = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        candidate,
        Arc::new(JudgeClient {
            result: Ok(response.into()),
        }),
    )
    .await
    .unwrap();
    let (evaluation, outputs) = prepared.into_staged_parts();

    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        b"malformed live sentinel"
    );
    assert!(!tasks.join(ACCEPTANCE_LOCK_FILE).exists());
    assert!(!acceptance_pin_path(temp.path(), &tasks).exists());
    assert!(evaluation.findings.is_empty());
    assert_eq!(
        outputs
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        [
            "acceptance-contract.json",
            "acceptance-contract.lock",
            "acceptance-pin.json"
        ]
    );
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&outputs[0].1).unwrap();
    assert_eq!(
        contract.acceptance[0].judgment.verdict,
        archon_workflow::task_set_contract::JudgeDecision::Accepted
    );
}
