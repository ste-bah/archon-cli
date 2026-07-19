use super::workflow_live_v2_lifecycle_verify_options::{
    prepare_verification_items, verification_options, write_wave_parallelism,
};

fn record_series_contract() -> serde_json::Value {
    serde_json::json!({
        "kind": "required_universe_registry",
        "artifact_path": ".archon/demo/coverage.json",
        "registry_path": ".archon/demo/registry.json",
        "required_universe": true,
        "data_kind": "record_series",
        "universe_fields": ["instruments", "timeframes"],
        "cells_field": "cells",
        "cell_identity_fields": ["canonical_instrument", "timeframe"],
        "required_true_fields": ["available", "production_eligible"],
        "required_nonempty_fields": ["provider_symbol", "dataset_id", "version"],
        "positive_count_fields": ["row_count"],
        "gaps_field": "gaps",
        "registry_records_field": "datasets",
        "registry_key_fields": ["dataset_id", "version"],
        "registry_required_true_fields": ["production_eligible"],
        "registry_status_field": "status",
        "registry_allowed_statuses": ["Healthy"],
        "registry_count_field": "rows",
        "registry_identity_fields": {
            "canonical_instrument": "symbol",
            "timeframe": "timeframe"
        },
        "payload_path_field": "payload_path",
        "payload_format": "jsonl",
        "required_fields": ["timestamp", "value", "measure"],
        "non_constant_fields": ["value", "measure"],
        "series_value_fields": ["value", "measure"],
        "series_overlap_min_rows": 2,
        "request_path_field": "request_path",
        "requested_count_field": "count",
        "response_path_field": "response_path",
        "response_identity_fields": {
            "provider_symbol": "symbol",
            "timeframe": "timeframe"
        },
        "validation_path_field": "validation_path",
        "validation_status_field": "status",
        "validation_checks_field": "checks",
        "validation_check_status_field": "status",
        "validation_failed_values": ["failed"],
        "validation_passed_values": ["passed"]
    })
}

fn write_json(path: &std::path::Path, value: &serde_json::Value) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    std::fs::write(path, value.to_string()).expect("JSON artifact");
}

fn write_jsonl(path: &std::path::Path, rows: &[serde_json::Value]) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    let body = rows
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, format!("{body}\n")).expect("JSONL artifact");
}

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
        "deliverable_contracts": [record_series_contract()]
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
            .contains("payload field is constant or absent")
    );
    assert!(
        substantive["focused_verification"]
            .as_str()
            .expect("command")
            .contains("internally inconsistent")
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
fn neutral_declared_verifier_executes_for_clean_contract_driven_series() {
    let project = tempfile::tempdir().expect("project");
    let artifact_path = project.path().join(".archon/demo/coverage.json");
    let registry_path = project.path().join(".archon/demo/registry.json");
    let artifact = serde_json::json!({
        "instruments": ["ALPHA", "BETA"],
        "timeframes": ["1D"],
        "cells": [
            {
                "canonical_instrument": "ALPHA",
                "timeframe": "1D",
                "provider_symbol": "EXCHANGE:ALPHA",
                "available": true,
                "production_eligible": true,
                "row_count": 2,
                "dataset_id": "alpha",
                "version": "v1"
            },
            {
                "canonical_instrument": "BETA",
                "timeframe": "1D",
                "provider_symbol": "EXCHANGE:BETA",
                "available": true,
                "production_eligible": true,
                "row_count": 2,
                "dataset_id": "beta",
                "version": "v1"
            }
        ],
        "gaps": []
    });
    write_json(&artifact_path, &artifact);
    let validation = serde_json::json!({
        "status": "passed",
        "checks": [{"status": "passed"}]
    });
    for (name, symbol, rows) in [
        (
            "alpha",
            "ALPHA",
            vec![
                serde_json::json!({"timestamp": 1, "value": 10, "measure": 100}),
                serde_json::json!({"timestamp": 2, "value": 11, "measure": 110}),
            ],
        ),
        (
            "beta",
            "BETA",
            vec![
                serde_json::json!({"timestamp": 1, "value": 20, "measure": 200}),
                serde_json::json!({"timestamp": 2, "value": 22, "measure": 220}),
            ],
        ),
    ] {
        write_jsonl(
            &project
                .path()
                .join(format!(".archon/demo/{name}/payload.jsonl")),
            &rows,
        );
        write_json(
            &project
                .path()
                .join(format!(".archon/demo/{name}/request.json")),
            &serde_json::json!({"count": 2}),
        );
        write_json(
            &project
                .path()
                .join(format!(".archon/demo/{name}/response.json")),
            &serde_json::json!({"symbol": format!("EXCHANGE:{symbol}"), "timeframe": "1D"}),
        );
        write_json(
            &project
                .path()
                .join(format!(".archon/demo/{name}/validation.json")),
            &validation,
        );
    }
    write_json(
        &registry_path,
        &serde_json::json!({
            "datasets": {
              "alpha:v1": {
                "production_eligible": true,
                "status": "Healthy",
                "rows": 2,
                "symbol": "ALPHA",
                "timeframe": "1D",
                "payload_path": ".archon/demo/alpha/payload.jsonl",
                "request_path": ".archon/demo/alpha/request.json",
                "response_path": ".archon/demo/alpha/response.json",
                "validation_path": ".archon/demo/alpha/validation.json"
              },
              "beta:v1": {
                "production_eligible": true,
                "status": "Healthy",
                "rows": 2,
                "symbol": "BETA",
                "timeframe": "1D",
                "payload_path": ".archon/demo/beta/payload.jsonl",
                "request_path": ".archon/demo/beta/request.json",
                "response_path": ".archon/demo/beta/response.json",
                "validation_path": ".archon/demo/beta/validation.json"
              }
            }
        }),
    );
    let items = vec![serde_json::json!({
        "item_id": "verify-demo",
        "source_item_id": "implement-demo",
        "canonical_task_ids": ["TASK-DEMO-017"]
    })];
    let universe = serde_json::json!({"tasks": [{
        "canonical_task_id": "TASK-DEMO-017",
        "deliverable_contracts": [record_series_contract()]
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
}

#[test]
fn declared_gap_rows_do_not_require_healthy_dataset_references() {
    let project = tempfile::tempdir().expect("project");
    write_json(
        &project.path().join(".archon/demo/coverage.json"),
        &serde_json::json!({
            "instruments": ["ALPHA"],
            "timeframes": ["1D"],
            "cells": [{
                "canonical_instrument": "ALPHA",
                "timeframe": "1D",
                "provider_symbol": "EXCHANGE:ALPHA",
                "available": false,
                "production_eligible": false,
                "row_count": 0
            }],
            "gaps": [{
                "canonical_instrument": "ALPHA",
                "timeframe": "1D",
                "reason": "provider unavailable"
            }]
        }),
    );
    write_json(
        &project.path().join(".archon/demo/registry.json"),
        &serde_json::json!({"datasets": {}}),
    );
    let items = vec![serde_json::json!({
        "item_id": "verify-demo",
        "source_item_id": "implement-demo",
        "canonical_task_ids": ["TASK-DEMO-017"]
    })];
    let universe = serde_json::json!({"tasks": [{
        "canonical_task_id": "TASK-DEMO-017",
        "deliverable_contracts": [record_series_contract()]
    }]});
    let prepared = prepare_verification_items(items, project.path().to_str(), &[], &universe);
    let command = prepared
        .iter()
        .find(|item| item["item_id"] == "verify-TASK-DEMO-017-required-universe-registry")
        .and_then(|item| item["focused_verification"].as_str())
        .expect("generated verifier");

    let result = std::process::Command::new("/bin/zsh")
        .args(["-c", command])
        .output()
        .expect("execute verifier");
    let stdout = String::from_utf8_lossy(&result.stdout);

    assert!(!result.status.success());
    assert!(
        stdout.contains("declared deliverable contains 1 gap record"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("required non-empty field failed"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("has no declared registry record"),
        "{stdout}"
    );
}

#[test]
fn wf9_contaminated_fixture_replay_fails_substantive_contract() {
    let project = tempfile::tempdir().expect("project");
    let artifact_path = project.path().join(".archon/demo/coverage.json");
    let registry_path = project.path().join(".archon/demo/registry.json");
    write_json(
        &artifact_path,
        &serde_json::json!({
            "instruments": ["ALPHA", "BETA"],
            "timeframes": ["1D"],
            "cells": [
                {"canonical_instrument": "ALPHA", "timeframe": "1D", "provider_symbol": "EXCHANGE:ALPHA", "available": true, "production_eligible": true, "row_count": 3, "dataset_id": "alpha", "version": "v1"},
                {"canonical_instrument": "BETA", "timeframe": "1D", "provider_symbol": "EXCHANGE:BETA", "available": true, "production_eligible": true, "row_count": 3, "dataset_id": "beta", "version": "v1"}
            ],
            "gaps": []
        }),
    );
    let contaminated_alpha = vec![
        serde_json::json!({"timestamp": 1, "value": 5108.36, "measure": 0}),
        serde_json::json!({"timestamp": 2, "value": 5225.66, "measure": 0}),
        serde_json::json!({"timestamp": 3, "value": 5142.43, "measure": 0}),
    ];
    let contaminated_beta = vec![
        serde_json::json!({"timestamp": 10, "value": 5225.66, "measure": 0}),
        serde_json::json!({"timestamp": 11, "value": 5142.43, "measure": 0}),
        serde_json::json!({"timestamp": 12, "value": 5164.31, "measure": 0}),
    ];
    for (name, rows) in [("alpha", &contaminated_alpha), ("beta", &contaminated_beta)] {
        write_jsonl(
            &project
                .path()
                .join(format!(".archon/demo/{name}/payload.jsonl")),
            rows,
        );
        write_json(
            &project
                .path()
                .join(format!(".archon/demo/{name}/request.json")),
            &serde_json::json!({"count": 100}),
        );
        write_json(
            &project
                .path()
                .join(format!(".archon/demo/{name}/response.json")),
            &serde_json::json!({"symbol": "EXCHANGE:ALPHA", "timeframe": "1D"}),
        );
        write_json(
            &project
                .path()
                .join(format!(".archon/demo/{name}/validation.json")),
            &serde_json::json!({
                "status": "passed",
                "checks": [{"status": "failed"}]
            }),
        );
    }
    write_json(
        &registry_path,
        &serde_json::json!({
            "datasets": {
              "alpha:v1": {"production_eligible": true, "status": "Healthy", "rows": 3, "symbol": "ALPHA", "timeframe": "1D", "payload_path": ".archon/demo/alpha/payload.jsonl", "request_path": ".archon/demo/alpha/request.json", "response_path": ".archon/demo/alpha/response.json", "validation_path": ".archon/demo/alpha/validation.json"},
              "beta:v1": {"production_eligible": true, "status": "Healthy", "rows": 3, "symbol": "BETA", "timeframe": "1D", "payload_path": ".archon/demo/beta/payload.jsonl", "request_path": ".archon/demo/beta/request.json", "response_path": ".archon/demo/beta/response.json", "validation_path": ".archon/demo/beta/validation.json"}
            }
        }),
    );
    let items = vec![serde_json::json!({
        "item_id": "verify-demo",
        "source_item_id": "implement-demo",
        "canonical_task_ids": ["TASK-DEMO-017"]
    })];
    let universe = serde_json::json!({"tasks": [{
        "canonical_task_id": "TASK-DEMO-017",
        "deliverable_contracts": [record_series_contract()]
    }]});
    let prepared = prepare_verification_items(items, project.path().to_str(), &[], &universe);
    let command = prepared
        .iter()
        .find(|item| item["item_id"] == "verify-TASK-DEMO-017-required-universe-registry")
        .and_then(|item| item["focused_verification"].as_str())
        .expect("generated verifier");

    let failing = std::process::Command::new("/bin/zsh")
        .args(["-c", command])
        .output()
        .expect("execute contaminated verifier");
    let stdout = String::from_utf8_lossy(&failing.stdout);

    assert!(!failing.status.success());
    assert!(stdout.contains("internally inconsistent"), "{stdout}");
    assert!(stdout.contains("below requested count"), "{stdout}");
    assert!(stdout.contains("constant or absent"), "{stdout}");
    assert!(
        stdout.contains("share a declared payload-series window"),
        "{stdout}"
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
