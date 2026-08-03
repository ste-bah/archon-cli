use super::*;

pub(super) fn record_series_contract() -> serde_json::Value {
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

pub(super) fn write_json(path: &std::path::Path, value: &serde_json::Value) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    std::fs::write(path, value.to_string()).expect("JSON artifact");
}

pub(super) fn write_jsonl(path: &std::path::Path, rows: &[serde_json::Value]) {
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

pub(super) fn run_verifier(command: &str) -> std::process::Output {
    // `sh`, not `/bin/zsh`: production runs focused verifiers through
    // `Command::new(archon_shell::resolve_posix_shell())` (workflow_live_v2_verification.rs:287), as does every
    // other verifier path in the workspace. Executing them under a different
    // shell here meant the tests were not exercising what ships, and an
    // absolute /bin/zsh made them fail outright on any host without zsh —
    // passing in CI only because the ubuntu runner image happens to include it.
    // The generated commands are plain POSIX.
    // Fed on stdin, matching `run_contract_verifier`. The generated verifier
    // embeds a ~29 KB Python program and Windows truncates any command line
    // past 32,767 characters, which severed the heredoc mid-script.
    use std::io::Write as _;
    let mut child = std::process::Command::new(archon_shell::resolve_posix_shell())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn generated verifier");
    child
        .stdin
        .take()
        .expect("verifier stdin")
        .write_all(command.as_bytes())
        .expect("write verifier script");
    child
        .wait_with_output()
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
    let command = super::super::workflow_live_v2_deliverable_contract::verification_command(
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

/// The glob form of instance binding, and the floor that makes it a claim.
///
/// This test used to assert that a glob with no declared floor passes on zero
/// matches — "vacuous" was in its name. That is prior-run finding F4: a
/// deliverable reported present against a wildcard path nobody could have
/// written to. Under D3 the unbound form is refused outright, and the same
/// contract with `min_instances` declared behaves as it always did: zero matches
/// fails the floor, and a match that is internally inconsistent still fails.
#[test]
fn a_glob_bound_by_a_floor_validates_its_matches_and_unbound_is_refused() {
    let project = tempfile::tempdir().expect("project");
    let mut contract = serde_json::json!({
        "kind": "instance_report",
        "artifact_path": ".archon/demo/glob/<instance-id>/report.json",
        "validation_status_field": "status",
        "validation_checks_field": "checks",
        "validation_check_status_field": "status",
        "validation_failed_values": ["failed"],
        "validation_passed_values": ["passed"]
    });
    let unbound = super::super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let refused = run_verifier(&unbound);
    assert!(
        !refused.status.success(),
        "a glob with no declared floor can never fail, so it must not be run"
    );
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("<instance-id>"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );

    contract["min_instances"] = serde_json::json!(1);
    let command = super::super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let empty = run_verifier(&command);
    assert!(!empty.status.success(), "zero matches is below the floor");

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
