use super::*;

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

    let passing = run_verifier_script(command);
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

    let result = run_verifier_script(command);
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

    let failing = run_verifier_script(command);
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
pub(super) fn series_project(rows: Vec<serde_json::Value>) -> (tempfile::TempDir, serde_json::Value) {
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

pub(super) fn verifier_stdout(project: &tempfile::TempDir, contract: &serde_json::Value) -> String {
    let command = super::super::workflow_live_v2_deliverable_contract::verification_command(
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

/// Run a generated verifier by piping it to the shell's stdin.
///
/// Not `sh -c <script>`: the generated verifier embeds a ~29 KB Python program
/// and Windows truncates any command line beyond 32,767 characters, severing
/// the heredoc mid-statement. stdin has no such limit.
fn run_verifier_script(command: &str) -> std::process::Output {
    use std::io::Write as _;
    let mut child = std::process::Command::new(crate::command::posix_shell::posix_shell())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn verifier");
    child
        .stdin
        .take()
        .expect("verifier stdin")
        .write_all(command.as_bytes())
        .expect("write verifier script");
    child.wait_with_output().expect("execute verifier")
}
