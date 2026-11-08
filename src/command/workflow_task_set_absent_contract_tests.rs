use super::*;

#[test]
fn in_round_freeze_clears_floor_implements_and_keeps_other_entries_bytes() {
    use archon_workflow::task_set_contract::{AcceptanceCheck, AcceptanceContract};
    use archon_workflow::task_universe::WorkflowV2DeliverableContract;

    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, candidate) = seed(&temp);
    let mut contract: AcceptanceContract = serde_json::from_slice(&candidate).unwrap();
    let mut supplementary = contract.acceptance[0].clone();
    supplementary.id = "SUP-REQ-X-001".into();
    contract.supplementary.push(supplementary);
    let kept_acceptance = serde_json::to_vec_pretty(&contract.acceptance[0]).unwrap();
    let kept_supplementary = serde_json::to_vec_pretty(&contract.supplementary[0]).unwrap();
    for (entries, id, path) in [
        (&mut contract.acceptance, "AC-X-002", "output.json"),
        (&mut contract.supplementary, "SUP-REQ-X-002", "extra.json"),
    ] {
        let mut entry = entries[0].clone();
        entry.id = id.into();
        entry.check = AcceptanceCheck::Floor {
            contract: Box::new(WorkflowV2DeliverableContract {
                kind: "artifact".into(),
                artifact_path: path.into(),
                typed_verifier_command: Some("./verify-output".into()),
                implements: vec!["invented".into()],
                ..Default::default()
            }),
        };
        entries.push(entry);
    }
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

    assert!(!String::from_utf8_lossy(&prepared.contract_bytes).contains("invented"));
    let frozen: AcceptanceContract = serde_json::from_slice(&prepared.contract_bytes).unwrap();
    for entry in [&frozen.acceptance[1], &frozen.supplementary[1]] {
        let AcceptanceCheck::Floor { contract } = &entry.check else {
            panic!("expected floor");
        };
        assert!(contract.implements.is_empty());
    }
    assert_eq!(
        serde_json::to_vec_pretty(&frozen.acceptance[0]).unwrap(),
        kept_acceptance
    );
    assert_eq!(
        serde_json::to_vec_pretty(&frozen.supplementary[0]).unwrap(),
        kept_supplementary
    );
    assert_eq!(
        prepared.candidate_digest,
        content_digest(&prepared.contract_bytes)
    );
}

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
