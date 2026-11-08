use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};
use archon_workflow::task_skeleton::TaskSkeleton;

#[test]
fn existing_skeleton_bytes_and_digest_survive_optional_status() {
    let bytes = br#"{"schema_version":1,"acceptance_digest":"frozen","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","depends_on":[],"blocks":[],"implements":["AC-X-001"],"deliverable_contracts":[]}]}"#;
    let mut skeleton: TaskSkeleton = serde_json::from_slice(bytes).unwrap();
    assert_eq!(skeleton.tasks[0].status, None);
    let roundtrip = serde_json::to_vec(&skeleton).unwrap();
    assert_eq!(roundtrip, bytes);
    assert_eq!(content_digest(&roundtrip), content_digest(bytes));
    skeleton.tasks[0].status = Some("done".into());
    let roundtrip: TaskSkeleton =
        serde_json::from_slice(&serde_json::to_vec(&skeleton).unwrap()).unwrap();
    assert_eq!(roundtrip.tasks[0].status.as_deref(), Some("done"));
}

#[test]
fn existing_acceptance_floor_bytes_and_digest_survive_optional_implements() {
    let bytes = br#"{"schema_version":1,"prd":{"path":"input.md","digest":"frozen"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":[{"id":"AC-X-001","criterion":"output exists","check":{"kind":"floor","contract":{"kind":"report","artifact_path":"output.json","min_instances":0,"required_universe":false,"registry_minimum_count":0,"series_overlap_min_rows":0}},"gap_permitted":false,"judgment":{"verdict":"accepted","counterexample":"absent output","reason":"checks output","host_call_id":"recorded"}}],"supplementary":[]}"#;
    let mut contract: AcceptanceContract = serde_json::from_slice(bytes).unwrap();
    let roundtrip = serde_json::to_vec(&contract).unwrap();
    assert_eq!(roundtrip, bytes);
    assert_eq!(content_digest(&roundtrip), content_digest(bytes));
    let archon_workflow::task_set_contract::AcceptanceCheck::Floor { contract: floor } =
        &mut contract.acceptance[0].check
    else {
        panic!("expected floor")
    };
    assert!(floor.implements.is_empty());
    floor.implements = vec!["item".into()];
    let roundtrip: AcceptanceContract =
        serde_json::from_slice(&serde_json::to_vec(&contract).unwrap()).unwrap();
    assert_eq!(roundtrip, contract);
}

#[test]
fn deliverable_contract_still_denies_unknown_fields() {
    assert!(
        serde_json::from_str::<archon_workflow::task_universe::WorkflowV2DeliverableContract>(
            r#"{"kind":"report","artifact_path":"output.json","unknown":true}"#
        )
        .is_err()
    );
}

#[test]
fn positional_inputs_keep_existing_field_order_and_default_new_tail_fields() {
    use archon_workflow::task_skeleton::FrozenTask;
    use archon_workflow::task_universe::WorkflowV2DeliverableContract;
    use serde_json::json;
    let mut task = json!(["unit", "unit.md", [], [], ["item"], [], []]);
    let frozen: FrozenTask = serde_json::from_value(task.clone()).unwrap();
    assert_eq!(frozen.task_id, "unit");
    assert_eq!(frozen.status, None);
    task.as_array_mut().unwrap().push(json!("done"));
    let frozen: FrozenTask = serde_json::from_value(task).unwrap();
    assert_eq!(frozen.status.as_deref(), Some("done"));

    let mut contract = json!([
        "output",
        "out",
        null,
        null,
        null,
        null,
        null,
        0,
        false,
        null,
        [],
        null,
        [],
        [],
        [],
        [],
        {},
        null,
        null,
        [],
        [],
        null,
        [],
        null,
        0,
        {},
        null,
        null,
        [],
        [],
        null,
        null,
        null,
        [],
        null,
        null,
        [],
        0,
        null,
        null,
        null,
        {},
        null,
        null,
        null,
        null,
        [],
        []
    ]);
    let frozen: WorkflowV2DeliverableContract = serde_json::from_value(contract.clone()).unwrap();
    assert_eq!(frozen.artifact_path, "out");
    assert!(frozen.implements.is_empty());
    contract.as_array_mut().unwrap().push(json!(["item"]));
    let frozen: WorkflowV2DeliverableContract = serde_json::from_value(contract).unwrap();
    assert_eq!(frozen.implements, ["item"]);
}
