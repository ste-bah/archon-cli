use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/d33_verification_remediation_contract_failure.json"
    ))
    .expect("D33 fixture")
}

#[test]
fn d33_size_gate_failure_is_repairable_contract_output() {
    let fixture = fixture();
    let repairable = workflow_live_v2_lifecycle_verify_outcome_repair::repairable_contract_outcomes(
        &fixture["remediation_wave"],
    );

    assert_eq!(repairable.len(), 1);
    assert_eq!(repairable[0]["failure_kind"], "contract");
    assert!(
        repairable[0]["evidence"][0]["summary"]
            .as_str()
            .unwrap()
            .contains("exceeds max 500")
    );
}

#[test]
fn d33_safety_failure_is_not_mechanically_repaired() {
    let wave = serde_json::json!({
        "outcomes": [{
            "item_id": "unsafe-write",
            "status": "failed",
            "failure_kind": "safety"
        }]
    });

    let repairable =
        workflow_live_v2_lifecycle_verify_outcome_repair::repairable_contract_outcomes(&wave);

    assert!(repairable.is_empty());
}

#[test]
fn d33_followup_replaces_only_the_failed_remediation_outcome() {
    let fixture = fixture();
    let followup_items = support::array(fixture["followup_inventory"].get("items"));
    let merged = workflow_live_v2_lifecycle_verify_outcome_repair::merge_repaired_outcomes(
        &fixture["remediation_wave"],
        fixture["followup_wave"].clone(),
        &followup_items,
    );
    let outcomes = support::outcomes_of(&merged);

    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(support::outcome_accepted_or_noop));
    assert!(outcomes.iter().any(|outcome| {
        outcome["item_id"] == "accepted-sibling" && outcome["status"] == "accepted"
    }));
    assert!(outcomes.iter().any(|outcome| {
        outcome["source_item_id"] == "failed-provider-envelope" && outcome["status"] == "accepted"
    }));
}

#[test]
fn d33_prompt_requires_split_or_retry_for_mechanical_contract_failure() {
    let prompt = workflow_live_v2_lifecycle_prompts::REMEDIATION_OUTCOME_REPAIR_TASK;

    assert!(prompt.contains("size/format/complexity"));
    assert!(prompt.contains("split"));
    assert!(prompt.contains("500"));
}
