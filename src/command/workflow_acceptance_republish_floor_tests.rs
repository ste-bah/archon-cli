use super::test_fixture::{criterion, frozen_set, write_chain};
use super::*;
use archon_workflow::task_set_contract::AcceptanceCheck;
use archon_workflow::task_universe::WorkflowV2DeliverableContract;

fn floor(path: &str, implements: Vec<String>) -> AcceptanceCheck {
    AcceptanceCheck::Floor {
        contract: Box::new(WorkflowV2DeliverableContract {
            kind: "artifact".into(),
            artifact_path: path.into(),
            typed_verifier_command: Some("./verify-output".into()),
            implements,
            ..Default::default()
        }),
    }
}

#[test]
fn republish_clears_floor_implements_and_keeps_other_entries_bytes() {
    let set = frozen_set(&[
        ("AC-F-001", "./verify-output", true),
        ("AC-F-002", "./verify-output", true),
    ]);
    let mut original = set.contract();
    original.acceptance[1].check = floor("kept.json", Vec::new());
    let mut supplementary = criterion("SUP-REQ-F-001", "./verify-output", true);
    supplementary.check = floor("extra.json", Vec::new());
    original.supplementary.push(supplementary);
    write_chain(
        set.project.path(),
        &set.tasks,
        &original,
        FreezeGateMode::Observe,
        0,
    );
    let kept = serde_json::to_vec_pretty(&original.acceptance[1]).unwrap();
    let named = BTreeSet::from(["AC-F-001".into(), "SUP-REQ-F-001".into()]);
    let request = ReauthorRequest {
        project_root: set.project.path(),
        tasks_root: &set.tasks,
        prd_path: &set.prd,
        ids: &named,
        gate: set.gate(),
        trigger: "test",
    };
    let verified = verify(&request).unwrap();
    let mut repaired = original.clone();
    repaired.acceptance[0].check = floor("repaired.json", vec!["invented".into()]);
    let AcceptanceCheck::Floor { contract } = &mut repaired.supplementary[0].check else {
        panic!("expected floor");
    };
    contract.implements.push("invented".into());
    let result = publish_chain(&request, verified, repaired, Vec::new(), None, &named).unwrap();
    assert!(result.published);
    let published = set.contract();
    for entry in [&published.acceptance[0], &published.supplementary[0]] {
        let AcceptanceCheck::Floor { contract } = &entry.check else {
            panic!("expected floor");
        };
        assert!(contract.implements.is_empty());
    }
    assert!(
        !String::from_utf8(set.contract_bytes())
            .unwrap()
            .contains("invented")
    );
    assert_eq!(
        serde_json::to_vec_pretty(&published.acceptance[1]).unwrap(),
        kept
    );
}
