use super::workflow_live_v2_lifecycle_verify_options::{
    prepare_verification_items, verification_options, write_wave_parallelism,
};

#[test]
fn verification_items_receive_the_runtime_project_root() {
    let items = vec![serde_json::json!({
        "item_id": "verify-artifact",
        "artifact_requirements": [".archon/data/validation.json"]
    })];

    let prepared = prepare_verification_items(items, Some("/runtime/project"), &[]);

    assert_eq!(prepared[0]["project_artifact_root"], "/runtime/project");
}

#[test]
fn verification_items_receive_manifest_grounded_diff_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = temp
        .path()
        .join("write-coordination/stages/write/manifests/branch.json");
    std::fs::create_dir_all(manifest_path.parent().expect("manifest parent"))
        .expect("manifest dir");
    std::fs::write(
        &manifest_path,
        serde_json::json!({
            "schema": "archon.workflow.patch_manifest.v1",
            "item_id": "write-task",
            "declared_target_files": ["src/owned.rs"],
            "changed_files": ["src/owned.rs"]
        })
        .to_string(),
    )
    .expect("manifest");
    let items = vec![serde_json::json!({
        "item_id": "verify-scope",
        "source_item_id": "write-task",
        "canonical_task_ids": ["TASK-001"]
    })];
    let evidence = vec![serde_json::json!({
        "result": { "data": { "outcomes": [{
            "item_id": "write-task",
            "canonical_task_ids": ["TASK-001"],
            "completion_evidence": [{
                "artifact_paths": [manifest_path]
            }]
        }] } }
    })];

    let prepared = prepare_verification_items(items, None, &evidence);

    assert_eq!(
        prepared[0]["write_coordination_scope"]["declared_target_files"],
        serde_json::json!(["src/owned.rs"])
    );
    assert_eq!(
        prepared[0]["write_coordination_scope"]["changed_files"],
        serde_json::json!(["src/owned.rs"])
    );
}

#[test]
fn d40_tdl_080_gets_substantive_registry_backed_coverage_verification() {
    let items = vec![serde_json::json!({
        "item_id": "verify-TASK-TDL-080-shape-only",
        "source_item_id": "impl-TASK-TDL-080-coverage",
        "canonical_task_ids": ["TASK-TDL-080"],
        "focused_verification": "jq '.cells | length' coverage/latest.json"
    })];

    let prepared = prepare_verification_items(items, Some("/runtime/project"), &[]);
    let substantive = prepared
        .iter()
        .find(|item| item["item_id"] == "verify-TASK-TDL-080-required-native-coverage")
        .expect("substantive coverage check");

    assert!(
        substantive["focused_verification"]
            .as_str()
            .expect("command")
            .contains("lacks healthy registered native provenance")
    );
    assert_eq!(
        substantive["provider_env_requirements"],
        serde_json::json!(["POLYGON_API_KEY"])
    );
    assert_eq!(
        substantive["required_tools"],
        serde_json::json!(["mcp__tradingview__data_get_ohlcv"])
    );
}

#[test]
fn cargo_verification_waves_are_serialized() {
    let items = vec![serde_json::json!({
        "item_id": "verify-cargo",
        "focused_verification": ["cargo test focused"]
    })];

    let options = verification_options(&items, "verify", true);

    assert_eq!(options["maxParallelism"], 1);
}

#[test]
fn non_cargo_verification_keeps_default_parallelism() {
    let items = vec![serde_json::json!({
        "item_id": "verify-python",
        "focused_verification": ["python3 check.py"]
    })];

    let options = verification_options(&items, "verify", true);

    assert!(options.get("maxParallelism").is_none());
}

#[test]
fn cargo_write_waves_serialize_before_agent_launch() {
    let items = vec![serde_json::json!({
        "item_id": "write-cargo",
        "focused_verification": ["cargo test focused"]
    })];

    assert_eq!(write_wave_parallelism(&items), 1);
}
