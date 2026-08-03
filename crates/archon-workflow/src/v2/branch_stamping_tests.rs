use crate::task_universe::WorkflowV2TaskUniverse;

fn universe() -> WorkflowV2TaskUniverse {
    serde_json::from_value(serde_json::json!({
        "schema_version": "v1",
        "source_roots": ["tasks"],
        "tasks": [{
            "canonical_task_id": "TASK-EX-001",
            "source_path": "tasks/TASK-EX-001.md",
            "required_tools": ["read_tool", "probe_tool"],
            "deliverable_contracts": [{
                "kind": "record_series",
                "artifact_path": ".archon/demo/coverage.json"
            }]
        }, {
            "canonical_task_id": "TASK-EX-002",
            "source_path": "tasks/TASK-EX-002.md"
        }]
    }))
    .expect("task universe")
}

fn item(id: &str, task_ids: serde_json::Value) -> super::WorkflowV2FanoutItem {
    super::WorkflowV2FanoutItem::read_only(
        id,
        "verifier",
        crate::v2::WorkflowV2HostCall {
            id: "verification-wave".to_string(),
            method: crate::v2::WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options: crate::v2::WorkflowV2HostOptions::default(),
        },
        serde_json::json!({
            "item": {"item_id": id, "canonical_task_ids": task_ids},
            "_workflow_project_artifact_policy": {"project_root": "/proj"}
        }),
    )
}

/// The v3 authored prelude builds its own verification item and never
/// attaches a contract, so the host verifier had nothing to enforce and
/// silently passed every branch. The universe is the authority.
#[test]
fn a_v3_verification_item_is_bound_to_its_tasks_declared_contracts() {
    let items = super::stamp_declared_contracts_from_universe(
        vec![item(
            "verify-task-ex-001",
            serde_json::json!(["TASK-EX-001"]),
        )],
        Some(&universe()),
    );
    let declared = super::declared_contracts_by_item(&items);
    let (root, contracts) = declared
        .get("verify-task-ex-001")
        .expect("contract must be bound");
    assert_eq!(root, "/proj");
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0]["artifact_path"], ".archon/demo/coverage.json");
}

/// A verifier asked to prove live tool invocations must be able to make
/// them. Write branches and the decomposed path bound declared tools; the
/// v3 authored path did not, so tasks whose acceptance requires tool calls
/// were unverifiable by construction.
#[test]
fn a_v3_verification_item_is_granted_its_tasks_declared_tools() {
    let items = super::stamp_required_tools_from_universe(
        vec![item(
            "verify-task-ex-001",
            serde_json::json!(["TASK-EX-001"]),
        )],
        Some(&universe()),
    );
    let tools = items[0].input["item"]["required_tools"]
        .as_array()
        .expect("tools must be stamped");
    let names: Vec<&str> = tools.iter().filter_map(|t| t.as_str()).collect();
    assert_eq!(names, vec!["probe_tool", "read_tool"]);
}

/// Universe-sourced: a branch claiming a task that declares no tools gets
/// none, so this cannot become a backdoor grant.
#[test]
fn a_task_declaring_no_tools_grants_none() {
    let items = super::stamp_required_tools_from_universe(
        vec![item(
            "verify-task-ex-002",
            serde_json::json!(["TASK-EX-002"]),
        )],
        Some(&universe()),
    );
    assert!(items[0].input["item"].get("required_tools").is_none());
}

#[test]
fn a_branch_claiming_no_task_is_left_alone() {
    let items = super::stamp_declared_contracts_from_universe(
        vec![item("adversarial-review-map-0", serde_json::json!([]))],
        Some(&universe()),
    );
    assert!(items[0].input.get("deliverable_contracts").is_none());
    assert!(super::declared_contracts_by_item(&items).is_empty());
}

/// The decomposed path stamps a singular `deliverable_contract` per item;
/// re-stamping would overwrite the contract that path deliberately chose.
#[test]
fn a_contract_already_stamped_by_the_decomposed_path_is_preserved() {
    let mut existing = item(
        "verify-TASK-EX-001-kind",
        serde_json::json!(["TASK-EX-001"]),
    );
    existing.input.as_object_mut().expect("object").insert(
        "deliverable_contract".to_string(),
        serde_json::json!({"kind": "chosen", "artifact_path": ".archon/chosen.json"}),
    );
    let items = super::stamp_declared_contracts_from_universe(vec![existing], Some(&universe()));
    assert!(items[0].input.get("deliverable_contracts").is_none());
    let declared = super::declared_contracts_by_item(&items);
    let (_, contracts) = declared.get("verify-TASK-EX-001-kind").expect("bound");
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0]["kind"], "chosen");
}

/// Without an artifact root the contract's relative paths cannot resolve,
/// so there is nothing meaningful to verify against.
#[test]
fn a_branch_without_an_artifact_root_is_not_enforced() {
    let mut orphan = item("verify-task-ex-001", serde_json::json!(["TASK-EX-001"]));
    orphan
        .input
        .as_object_mut()
        .expect("object")
        .remove("_workflow_project_artifact_policy");
    let items = super::stamp_declared_contracts_from_universe(vec![orphan], Some(&universe()));
    assert!(items[0].input.get("deliverable_contracts").is_some());
    assert!(super::declared_contracts_by_item(&items).is_empty());
}
