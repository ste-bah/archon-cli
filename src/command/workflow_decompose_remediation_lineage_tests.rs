use super::*;

#[test]
fn self_heal_reauthors_reverted_entries_but_skips_entries_equal_to_the_repaired_lineage_version() {
    let ids = (1..=5)
        .map(|index| format!("AC-F-{index:03}"))
        .collect::<Vec<_>>();
    let checks = ids
        .iter()
        .map(|id| (id.as_str(), "grep -q ready out.txt", true))
        .collect::<Vec<_>>();
    let set =
        crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&checks);
    let contract_path = set
        .tasks
        .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let before_bytes = std::fs::read(&contract_path).unwrap();
    let before: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&before_bytes).unwrap();
    let mut after = before.clone();
    for entry in &mut after.acceptance {
        entry.check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
            command: "grep -q ready out.txt && test -s out.txt".into(),
            cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
        };
    }
    let after_bytes = serde_json::to_vec_pretty(&after).unwrap();
    let pin_path = set.pin_path();
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    let before_digest = archon_workflow::task_set_contract::content_digest(&before_bytes);
    let after_digest = archon_workflow::task_set_contract::content_digest(&after_bytes);
    let history = archon_workflow::task_set_lineage::ChainHistory::for_pin(&pin_path);
    history.put(&before_bytes).unwrap();
    history.put(&after_bytes).unwrap();
    let mut successor = pin.clone();
    successor.acceptance_digest = after_digest.clone();
    successor.freeze_event_id = "acceptance-freeze-successor".into();
    pin.lineage = vec![archon_workflow::task_set_lineage::PinTransition::extending(
        &[],
        pin.identity(),
        successor.identity(),
        ids.iter().cloned().collect(),
        "test reauthor",
    )];
    pin.acceptance_digest = after_digest.clone();
    pin.freeze_event_id = successor.freeze_event_id;
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    // The live bytes match the transition's replaced version.
    std::fs::write(&contract_path, &before_bytes).unwrap();
    let ledger = Ledger {
        report_id: "consumed-report".into(),
        subjects: ids.clone(),
    };
    let findings = consumed_ledger_findings(set.project.path(), &set.tasks, &ledger).unwrap();
    assert_eq!(findings.len(), 5);
    for id in &ids {
        assert!(findings[id].contains("prove a reverted entry"));
        assert!(findings[id].contains(&before_digest));
        assert!(findings[id].contains(&after_digest));
    }

    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    std::fs::write(&contract_path, &after_bytes).unwrap();
    assert!(
        consumed_ledger_findings(set.project.path(), &set.tasks, &ledger)
            .unwrap()
            .is_empty()
    );
}
