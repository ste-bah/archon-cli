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

fn parameterized_source_contract() -> serde_json::Value {
    serde_json::json!({
        "kind": "instance_report",
        "artifact_path": ".archon/demo/instances/<instance-id>/report.json",
        "instance_source_path": ".archon/demo/instances.json",
        "instance_source_records_field": "records",
        "instance_artifact_field": "report_path",
        "validation_status_field": "status",
        "validation_checks_field": "checks",
        "validation_check_status_field": "status",
        "validation_failed_values": ["failed"],
        "validation_passed_values": ["passed"]
    })
}

fn parameterized_verifier(project: &std::path::Path) -> String {
    let items = vec![serde_json::json!({
        "item_id": "verify-instance-source",
        "source_item_id": "implement-instance-source",
        "canonical_task_ids": ["TASK-DEMO-INSTANCE"]
    })];
    let universe = serde_json::json!({"tasks": [{
        "canonical_task_id": "TASK-DEMO-INSTANCE",
        "deliverable_contracts": [parameterized_source_contract()]
    }]});
    prepare_verification_items(items, project.to_str(), &[], &universe)
        .into_iter()
        .find(|item| item["item_id"] == "verify-TASK-DEMO-INSTANCE-instance-report")
        .and_then(|item| item["focused_verification"].as_str().map(str::to_string))
        .expect("generated parameterized verifier")
}

fn run_verifier(command: &str) -> std::process::Output {
    std::process::Command::new("/bin/zsh")
        .args(["-c", command])
        .output()
        .expect("execute generated verifier")
}

#[test]
fn parameterized_source_contract_covers_empty_valid_missing_and_inconsistent_instances() {
    let project = tempfile::tempdir().expect("project");
    let source = project.path().join(".archon/demo/instances.json");
    let valid_report = project
        .path()
        .join(".archon/demo/instances/alpha/report.json");
    let inconsistent_report = project
        .path()
        .join(".archon/demo/instances/beta/report.json");
    let missing_report = ".archon/demo/instances/missing/report.json";
    let command = parameterized_verifier(project.path());

    write_json(
        &inconsistent_report,
        &serde_json::json!({
            "status": "passed",
            "checks": [{"status": "failed"}]
        }),
    );
    write_json(&source, &serde_json::json!({"records": {}}));
    let empty = run_verifier(&command);
    assert!(
        empty.status.success(),
        "{}{}",
        String::from_utf8_lossy(&empty.stdout),
        String::from_utf8_lossy(&empty.stderr)
    );
    assert!(
        String::from_utf8_lossy(&empty.stdout).contains("\"instance_count\": 0"),
        "{}",
        String::from_utf8_lossy(&empty.stdout)
    );

    write_json(
        &valid_report,
        &serde_json::json!({
            "status": "passed",
            "checks": [{"status": "passed"}]
        }),
    );
    write_json(
        &source,
        &serde_json::json!({"records": {
            "alpha": {"report_path": ".archon/demo/instances/alpha/report.json"}
        }}),
    );
    let valid = run_verifier(&command);
    assert!(
        valid.status.success(),
        "{}{}",
        String::from_utf8_lossy(&valid.stdout),
        String::from_utf8_lossy(&valid.stderr)
    );

    write_json(
        &source,
        &serde_json::json!({"records": {
            "missing": {"report_path": missing_report}
        }}),
    );
    let missing = run_verifier(&command);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stdout).contains("missing or empty"),
        "{}",
        String::from_utf8_lossy(&missing.stdout)
    );

    write_json(
        &source,
        &serde_json::json!({"records": {
            "beta": {"report_path": ".archon/demo/instances/beta/report.json"}
        }}),
    );
    let inconsistent = run_verifier(&command);
    assert!(!inconsistent.status.success());
    assert!(
        String::from_utf8_lossy(&inconsistent.stdout).contains("internally inconsistent"),
        "{}",
        String::from_utf8_lossy(&inconsistent.stdout)
    );
}

#[test]
fn parameterized_contract_honors_min_instances() {
    let project = tempfile::tempdir().expect("project");
    write_json(
        &project.path().join(".archon/demo/instances.json"),
        &serde_json::json!({"records": {}}),
    );
    let mut contract = parameterized_source_contract();
    contract["min_instances"] = serde_json::json!(1);
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );

    let result = run_verifier(&command);

    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("requires >= 1 instance"),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
}

#[test]
fn parameterized_glob_fallback_is_vacuous_and_validates_matches() {
    let project = tempfile::tempdir().expect("project");
    let contract = serde_json::json!({
        "kind": "instance_report",
        "artifact_path": ".archon/demo/glob/<instance-id>/report.json",
        "validation_status_field": "status",
        "validation_checks_field": "checks",
        "validation_check_status_field": "status",
        "validation_failed_values": ["failed"],
        "validation_passed_values": ["passed"]
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );

    assert!(run_verifier(&command).status.success());
    write_json(
        &project.path().join(".archon/demo/glob/alpha/report.json"),
        &serde_json::json!({
            "status": "passed",
            "checks": [{"status": "failed"}]
        }),
    );
    let inconsistent = run_verifier(&command);
    assert!(!inconsistent.status.success());
    assert!(
        String::from_utf8_lossy(&inconsistent.stdout).contains("internally inconsistent"),
        "{}",
        String::from_utf8_lossy(&inconsistent.stdout)
    );
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

    let passing = std::process::Command::new("sh")
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

    let result = std::process::Command::new("sh")
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

    let failing = std::process::Command::new("sh")
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

/// Build a project whose payload rows are supplied by the caller, so a test can
/// choose exactly what the "observed" series looks like.
fn series_project(rows: Vec<serde_json::Value>) -> (tempfile::TempDir, serde_json::Value) {
    let project = tempfile::tempdir().expect("project");
    let count = rows.len();
    write_json(
        &project.path().join(".archon/demo/coverage.json"),
        &serde_json::json!({
            "instruments": ["ALPHA"],
            "timeframes": ["1D"],
            "cells": [{"canonical_instrument": "ALPHA", "timeframe": "1D",
                       "provider_symbol": "EXCHANGE:ALPHA", "available": true,
                       "production_eligible": true, "row_count": count,
                       "dataset_id": "alpha", "version": "v1"}],
            "gaps": []
        }),
    );
    write_jsonl(
        &project.path().join(".archon/demo/alpha/payload.jsonl"),
        &rows,
    );
    write_json(
        &project.path().join(".archon/demo/alpha/request.json"),
        &serde_json::json!({"count": count}),
    );
    write_json(
        &project.path().join(".archon/demo/alpha/response.json"),
        &serde_json::json!({"symbol": "EXCHANGE:ALPHA", "timeframe": "1D"}),
    );
    write_json(
        &project.path().join(".archon/demo/alpha/validation.json"),
        &serde_json::json!({"status": "passed", "checks": [{"status": "passed"}]}),
    );
    write_json(
        &project.path().join(".archon/demo/registry.json"),
        &serde_json::json!({"datasets": {"alpha:v1": {
            "production_eligible": true, "status": "Healthy", "rows": count,
            "symbol": "ALPHA", "timeframe": "1D",
            "payload_path": ".archon/demo/alpha/payload.jsonl",
            "request_path": ".archon/demo/alpha/request.json",
            "response_path": ".archon/demo/alpha/response.json",
            "validation_path": ".archon/demo/alpha/validation.json"}}}),
    );
    (project, record_series_contract())
}

fn verifier_stdout(project: &tempfile::TempDir, contract: &serde_json::Value) -> String {
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        contract,
    );
    String::from_utf8_lossy(&run_verifier(&command).stdout).into_owned()
}

/// Two alternating increments across 200 rows: the shape that was actually
/// fabricated. The old rule fired only when EVERY first difference matched, so
/// distinct == 2 walked straight through it.
#[test]
fn a_series_with_only_two_distinct_step_values_is_rejected_as_synthetic() {
    let rows: Vec<_> = (0..200)
        .map(|index| {
            let value = 100.0 + (index / 2) as f64 * 1.2 + (index % 2) as f64 * 0.5;
            serde_json::json!({"timestamp": 1_700_000_000i64 + index * 86_400,
                               "value": value, "measure": value * 2.0})
        })
        .collect();
    let (project, contract) = series_project(rows);
    let stdout = verifier_stdout(&project, &contract);
    assert!(stdout.contains("distinct first differences"), "{stdout}");
    assert!(stdout.contains("synthetic, not observed"), "{stdout}");
}

/// The counterpart guard: a check that misfires on genuine data would be worse
/// than no check, because everyone learns to ignore it. Real SPY closes score
/// ~0.88 on this ratio.
#[test]
fn an_irregular_series_of_the_same_length_is_accepted() {
    // A linear ramp plus a *linear* jitter is still synthetic -- its first
    // differences take two values, and an earlier draft of this fixture was
    // correctly rejected by the check. Real closes move independently each
    // session, so drive the walk with an LCG.
    let mut state: u64 = 12_345;
    let rows: Vec<_> = (0..200)
        .map(|index| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let step = ((state >> 33) % 10_000) as f64 / 1_000.0 - 5.0;
            let value = 500.0 + index as f64 * 0.1 + step;
            serde_json::json!({"timestamp": 1_700_000_000i64 + index * 86_400,
                               "value": value, "measure": value * 1.5 + step})
        })
        .collect();
    let (project, contract) = series_project(rows);
    let stdout = verifier_stdout(&project, &contract);
    assert!(!stdout.contains("distinct first differences"), "{stdout}");
    assert!(!stdout.contains("synthetic, not observed"), "{stdout}");
}

/// A venue that closes at weekends cannot have produced weekend records. The
/// forged set carried 48 of them.
#[test]
fn records_dated_to_a_closed_session_are_rejected() {
    // 1704067200 = 2024-01-01T00:00:00Z (Monday); +5d and +6d land on the weekend.
    let rows: Vec<_> = (0..40)
        .map(|index| {
            let jitter = ((index * 7919) % 977) as f64 / 100.0;
            serde_json::json!({"timestamp": 1_704_067_200i64 + index * 86_400,
                               "value": 100.0 + index as f64 * 0.4 + jitter,
                               "measure": 50.0 + index as f64 * 0.9 + jitter})
        })
        .collect();
    let (project, mut contract) = series_project(rows);
    contract["observed_time_field"] = serde_json::json!("timestamp");
    contract["closed_weekdays"] = serde_json::json!([5, 6]);
    contract["closed_dates"] = serde_json::json!(["2024-01-01"]);
    let stdout = verifier_stdout(&project, &contract);
    assert!(stdout.contains("when the venue was closed"), "{stdout}");
}

/// Without a declared calendar the check must stay silent -- it is opt-in, and
/// a 24/7 venue trades every day.
#[test]
fn a_venue_with_no_declared_calendar_keeps_every_session() {
    let rows: Vec<_> = (0..40)
        .map(|index| {
            let jitter = ((index * 7919) % 977) as f64 / 100.0;
            serde_json::json!({"timestamp": 1_704_067_200i64 + index * 86_400,
                               "value": 100.0 + index as f64 * 0.4 + jitter,
                               "measure": 50.0 + index as f64 * 0.9 + jitter})
        })
        .collect();
    let (project, mut contract) = series_project(rows);
    contract["observed_time_field"] = serde_json::json!("timestamp");
    let stdout = verifier_stdout(&project, &contract);
    assert!(!stdout.contains("when the venue was closed"), "{stdout}");
}

/// A markdown deliverable declared as `text` must pass on existence rather
/// than being parsed as JSON. Without this the verifier demoted correct work
/// permanently: no remediation can make prose parse, so the task looped to its
/// round cap and blocked on a defect that was in the contract, not the work.
#[test]
fn a_textual_deliverable_is_checked_for_presence_not_parsed_as_json() {
    let project = tempfile::tempdir().expect("project");
    let artifact = project.path().join(".archon/demo/inventory.md");
    std::fs::create_dir_all(artifact.parent().expect("parent")).expect("dir");
    std::fs::write(&artifact, "# Inventory\n\nRules are cited.\n").expect("artifact");
    let contract = serde_json::json!({
        "kind": "rule_inventory",
        "artifact_path": ".archon/demo/inventory.md",
        "artifact_format": "text"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.contains("declared_text_deliverable_present"),
        "{stdout}"
    );
}

/// Still fail-closed: declaring `text` buys presence, not a free pass.
#[test]
fn a_missing_textual_deliverable_still_fails() {
    let project = tempfile::tempdir().expect("project");
    let contract = serde_json::json!({
        "kind": "rule_inventory",
        "artifact_path": ".archon/demo/absent.md",
        "artifact_format": "text"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("missing or empty"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Inference must not weaken JSON validation: a `.json` deliverable with no
/// declared format is still strictly parsed and still fails when malformed.
#[test]
fn an_undeclared_json_extension_is_still_strictly_parsed() {
    let project = tempfile::tempdir().expect("project");
    let artifact = project.path().join(".archon/demo/thing.json");
    std::fs::create_dir_all(artifact.parent().expect("parent")).expect("dir");
    std::fs::write(&artifact, "# not json\n").expect("artifact");
    let contract = serde_json::json!({
        "kind": "thing",
        "artifact_path": ".archon/demo/thing.json"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("not valid JSON"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Site 2: the PARAMETERIZED/instance path. A markdown deliverable declared
/// with a `<placeholder>` segment goes down the instance branch, which loaded
/// every instance as JSON independently of the single-artifact branch. Fixing
/// only the single-artifact site left this one broken — and it is the one a
/// per-run report artifact actually travels through, so a helper-level test
/// would have reported the fix working while it still failed live.
#[test]
fn a_parameterized_markdown_instance_is_not_parsed_as_json() {
    let project = tempfile::tempdir().expect("project");
    let report = project.path().join(".archon/demo/runs/run-1/review.md");
    std::fs::create_dir_all(report.parent().expect("parent")).expect("dir");
    std::fs::write(&report, "# Adversarial review\n\nNo blocking issues.\n").expect("report");
    write_json(
        &project.path().join(".archon/demo/runs.json"),
        &serde_json::json!({"records": {"run-1": {"report_path": ".archon/demo/runs/run-1/review.md"}}}),
    );
    let contract = serde_json::json!({
        "kind": "run_review",
        "artifact_path": ".archon/demo/runs/<run-id>/review.md",
        "instance_source_path": ".archon/demo/runs.json",
        "instance_source_records_field": "records",
        "instance_artifact_field": "report_path"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "markdown instance must not be JSON-parsed: {stdout}"
    );
    assert!(!stdout.contains("not valid JSON"), "{stdout}");
}

/// Still fail-closed on the instance path: a declared instance that is missing
/// fails regardless of its extension.
#[test]
fn a_missing_parameterized_instance_still_fails() {
    let project = tempfile::tempdir().expect("project");
    write_json(
        &project.path().join(".archon/demo/runs.json"),
        &serde_json::json!({"records": {"run-1": {"report_path": ".archon/demo/runs/run-1/review.md"}}}),
    );
    let contract = serde_json::json!({
        "kind": "run_review",
        "artifact_path": ".archon/demo/runs/<run-id>/review.md",
        "instance_source_path": ".archon/demo/runs.json",
        "instance_source_records_field": "records",
        "instance_artifact_field": "report_path"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("missing or empty"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
