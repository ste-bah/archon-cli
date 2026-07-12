use super::workflow_live_v2_lifecycle_verify_options::{
    prepare_verification_items, verification_options,
};

#[test]
fn verification_items_receive_the_runtime_project_root() {
    let items = vec![serde_json::json!({
        "item_id": "verify-artifact",
        "artifact_requirements": [".archon/data/validation.json"]
    })];

    let prepared = prepare_verification_items(items, Some("/runtime/project"));

    assert_eq!(prepared[0]["project_artifact_root"], "/runtime/project");
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
