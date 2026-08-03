use super::*;

#[test]
fn workflow_live_generated_contract_normalizes_wf139e_verification_plan_fixture() {
    let fixture = archon_test_support::fixtures::WF139E_VERIFICATION_PLAN_ITEMS;
    let values: Vec<serde_json::Value> = serde_json::from_str(fixture).expect("fixture json");
    let normalized = values
        .iter()
        .map(|value| normalize_generated_item_value(value, Some(&tdl_task_universe())).value)
        .collect::<Vec<_>>();

    assert_eq!(normalized.len(), 4);
    assert!(
        normalized
            .iter()
            .all(|value| value["canonical_task_ids"][0] == "TASK-TDL-010")
    );
    assert!(normalized.iter().any(|value| {
        value["focused_verification"]
            .as_array()
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("command")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|command| command.contains("registry"))
                })
            })
    }));
    assert!(normalized.iter().any(|value| {
        value["artifact_requirements"]
            .as_array()
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.as_str()
                        .is_some_and(|path| path.contains("registry.json"))
                })
            })
    }));
}

#[test]
fn workflow_live_generated_contract_normalizes_wf139e_repair_plan_aliases() {
    for fixture in [
        archon_test_support::fixtures::WF139E_VERIFICATION_REPAIR_PLAN_1_1_ITEMS,
        archon_test_support::fixtures::WF139E_VERIFICATION_REPAIR_PLAN_1_2_ITEMS,
        archon_test_support::fixtures::WF139E_VERIFICATION_REPAIR_PLAN_1_3_ITEMS,
    ] {
        let values: Vec<serde_json::Value> = serde_json::from_str(fixture).expect("fixture json");
        let normalized = values
            .iter()
            .map(|value| normalize_generated_item_value(value, Some(&tdl_task_universe())).value)
            .collect::<Vec<_>>();

        assert!(
            normalized
                .iter()
                .all(|value| value["canonical_task_ids"][0] == "TASK-TDL-010")
        );
        assert!(
            normalized.iter().all(|value| value["focused_verification"]
                .as_array()
                .is_some_and(|items| !items.is_empty())),
            "{normalized:#?}"
        );
        assert!(
            normalized
                .iter()
                .all(|value| value.get("artifact_requirements").is_some()),
            "{normalized:#?}"
        );
    }
}

#[test]
fn workflow_live_generated_contract_treats_failure_kind_as_failure_evidence() {
    let value = serde_json::json!({
        "id": "repair-one",
        "source_item_id": "impl-one",
        "canonical_task_ids": ["TASK-TDL-010"],
        "status": "needs_review",
        "failure_kind": "verification_failed",
        "required_fix": "repair focused verification failure",
        "target_files": ["src/lib.rs"],
        "focused_verification": ["cargo test focused"],
        "artifact_requirements": [],
        "verification_requirements": ["cargo test focused"]
    });

    let normalized = normalize_generated_item_value(&value, Some(&tdl_task_universe())).value;

    assert_eq!(normalized["failure_status"], "needs_review");
    assert!(
        normalized["failure_evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item == "verification_failed"))
    );
}
