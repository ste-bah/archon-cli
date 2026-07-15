use super::workflow_live_v2_lifecycle_verify_options::{
    prepare_verification_items, verification_options, write_wave_parallelism,
};

#[test]
fn verification_items_receive_the_runtime_project_root() {
    let items = vec![serde_json::json!({
        "item_id": "verify-artifact",
        "artifact_requirements": [".archon/data/validation.json"]
    })];

    let prepared = prepare_verification_items(
        items,
        Some("/runtime/project"),
        &[],
        &serde_json::json!({"tasks": []}),
    );

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

    let prepared =
        prepare_verification_items(items, None, &evidence, &serde_json::json!({"tasks": []}));

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
fn declared_required_universe_gets_substantive_registry_backed_verification() {
    let items = vec![serde_json::json!({
        "item_id": "verify-TASK-DEMO-080-shape-only",
        "source_item_id": "impl-TASK-DEMO-080-coverage",
        "canonical_task_ids": ["TASK-DEMO-080"],
        "focused_verification": "jq '.cells | length' coverage/latest.json"
    })];
    let universe = serde_json::json!({"tasks": [{
        "canonical_task_id": "TASK-DEMO-080",
        "required_env_keys": ["DEMO_API_KEY"],
        "required_tools": ["mcp__demo__fetch_cells"],
        "deliverable_contracts": [{
            "kind": "required_universe_registry",
            "artifact_path": ".archon/demo/coverage.json",
            "registry_path": ".archon/demo/registry.json",
            "required_universe": true
        }]
    }]});

    let prepared = prepare_verification_items(items, Some("/runtime/project"), &[], &universe);
    let substantive = prepared
        .iter()
        .find(|item| item["item_id"] == "verify-TASK-DEMO-080-required-universe-registry")
        .expect("substantive coverage check");

    assert!(
        substantive["focused_verification"]
            .as_str()
            .expect("command")
            .contains("lacks healthy registered native provenance")
    );
    assert!(
        substantive["focused_verification"]
            .as_str()
            .expect("command")
            .contains("missing-symbol-or-interval")
    );
    assert_eq!(
        substantive["provider_env_requirements"],
        serde_json::json!(["DEMO_API_KEY"])
    );
    assert_eq!(
        substantive["required_tools"],
        serde_json::json!(["mcp__demo__fetch_cells"])
    );
}

#[test]
fn neutral_declared_verifier_executes_and_fails_closed_on_empty_cells() {
    let project = tempfile::tempdir().expect("project");
    let artifact_path = project.path().join(".archon/demo/coverage.json");
    let registry_path = project.path().join(".archon/demo/registry.json");
    std::fs::create_dir_all(artifact_path.parent().expect("artifact parent"))
        .expect("artifact directory");
    let artifact = serde_json::json!({
        "instruments": ["DEMO"],
        "timeframes": ["1D"],
        "cells": [{
            "canonical_instrument": "DEMO",
            "timeframe": "1D",
            "available": true,
            "native_interval": true,
            "production_eligible": true,
            "symbol": "DEMO",
            "interval": "1D",
            "row_count": 2,
            "dataset_id": "demo-native",
            "version": "v1"
        }],
        "gaps": []
    });
    std::fs::write(&artifact_path, artifact.to_string()).expect("artifact");
    std::fs::write(
        &registry_path,
        serde_json::json!({
            "datasets": {"demo-native:v1": {
                "native_interval": true,
                "production_eligible": true,
                "status": "Healthy",
                "bars": 2
            }}
        })
        .to_string(),
    )
    .expect("registry");
    let items = vec![serde_json::json!({
        "item_id": "verify-demo",
        "source_item_id": "implement-demo",
        "canonical_task_ids": ["TASK-DEMO-017"]
    })];
    let universe = serde_json::json!({"tasks": [{
        "canonical_task_id": "TASK-DEMO-017",
        "deliverable_contracts": [{
            "kind": "required_universe_registry",
            "artifact_path": ".archon/demo/coverage.json",
            "registry_path": ".archon/demo/registry.json",
            "required_universe": true
        }]
    }]});
    let prepared = prepare_verification_items(items, project.path().to_str(), &[], &universe);
    let command = prepared
        .iter()
        .find(|item| item["item_id"] == "verify-TASK-DEMO-017-required-universe-registry")
        .and_then(|item| item["focused_verification"].as_str())
        .expect("generated verifier");

    let passing = std::process::Command::new("/bin/zsh")
        .args(["-c", command])
        .output()
        .expect("execute verifier");
    assert!(
        passing.status.success(),
        "{}",
        String::from_utf8_lossy(&passing.stderr)
    );

    let mut empty = artifact;
    empty["cells"][0]["row_count"] = serde_json::json!(0);
    std::fs::write(&artifact_path, empty.to_string()).expect("empty artifact");
    let failing = std::process::Command::new("/bin/zsh")
        .args(["-c", command])
        .output()
        .expect("execute failing verifier");
    assert!(!failing.status.success());
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
