use super::*;
use archon_workflow::task_set_contract::JudgeDecision;

fn contract() -> AcceptanceContract {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "p", "digest": "d"},
        "gap_policy": {},
        "acceptance": [{
            "id": "AC-1", "criterion": "acceptance", "check": {"kind": "command", "command": "true", "cwd": "project_root"},
            "judgment": {"verdict": "accepted", "counterexample": "old counterexample", "reason": "old reason", "host_call_id": "old"}
        }],
        "supplementary": [{
            "id": "SUP-1", "criterion": "supplementary", "check": {"kind": "command", "command": "true", "cwd": "project_root"},
            "judgment": {"verdict": "refuted", "counterexample": "", "reason": "", "host_call_id": ""}
        }]
    }))
    .unwrap()
}

#[test]
fn merging_a_supplementary_subset_preserves_reused_acceptance_verdicts() {
    let mut contract = contract();
    let mut judged = contract.clone();
    judged.acceptance.clear();
    judged.supplementary[0].judgment.verdict = JudgeDecision::Accepted;
    judged.supplementary[0].judgment.counterexample = "no false pass".into();
    judged.supplementary[0].judgment.reason = "the criterion holds".into();
    judged.supplementary[0].judgment.host_call_id = "new".into();

    merge_judgments(&mut contract, &judged);

    assert_eq!(
        contract.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(contract.acceptance[0].judgment.host_call_id, "old");
    assert_eq!(
        contract.supplementary[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(contract.supplementary[0].judgment.host_call_id, "new");
}
