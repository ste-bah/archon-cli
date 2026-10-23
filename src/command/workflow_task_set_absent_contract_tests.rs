use super::*;

#[test]
fn in_round_freeze_prepares_when_the_live_contract_is_absent() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, candidate) = seed(&temp);
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&candidate).unwrap();
    std::fs::remove_file(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    let prepared = prepare_from_judged(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Enforce,
        &contract,
        Some("baseline".into()),
        capture_live_contract_digest(&tasks).unwrap(),
    )
    .unwrap();

    assert_eq!(prepared.live_contract_digest, None);
    assert_eq!(prepared.result.live_contract_digest, None);
    assert_eq!(
        prepared.candidate_digest,
        content_digest(&prepared.contract_bytes)
    );
}

#[test]
fn in_round_freeze_reports_a_corrupt_live_contract_path() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _prd, _candidate) = seed(&temp);
    let contract_path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    std::fs::write(&contract_path, b"not json").unwrap();
    let error = capture_live_contract_digest(&tasks).unwrap_err();

    assert!(format!("{error:#}").contains(&contract_path.display().to_string()));
}
