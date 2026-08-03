//! Declaration-driven capability merging, and the runtime-genericity gate.
//!
//! Split out of `workflow_live_task_universe_b.rs` only because each source
//! file in this tree is held under a 500-line ceiling; these are otherwise
//! ordinary tests of that module.

use super::*;

#[test]
fn neutral_task_and_project_capabilities_are_loaded_from_declarations() {
    let project = tempfile::tempdir().expect("project");
    let archon = project.path().join(".archon");
    let tasks = project.path().join("tasks/PRD-DEMO");
    fs::create_dir_all(&archon).expect("archon dir");
    fs::create_dir_all(&tasks).expect("tasks dir");
    fs::write(
        archon.join("project.json"),
        serde_json::json!({
            "required_env_keys": ["PROJECT_TOKEN"],
            "required_tools": ["project_probe"]
        })
        .to_string(),
    )
    .expect("project manifest");
    fs::write(
        tasks.join("TASK-DEMO-017-deliverable.md"),
        r#"# Neutral deliverable

```yaml
task_id: TASK-DEMO-017
title: Neutral deliverable
complexity: medium
status: ready
depends_on: []
blocks: []
implements: []
required_env_keys: [TASK_TOKEN]
required_tools: [fetch_demo]
deliverable_contracts:
  - kind: required_universe_registry
    artifact_path: .archon/demo/coverage.json
    registry_path: .archon/demo/registry.json
    instance_source_path: .archon/demo/instances.json
    instance_source_records_field: records
    instance_artifact_field: report_path
    min_instances: 2
    required_universe: true
    data_kind: record_series
    universe_fields: [instruments, intervals]
    cells_field: cells
    cell_identity_fields: [instrument, interval]
    required_true_fields: [available, eligible]
    required_nonempty_fields: [dataset_id, version]
    positive_count_fields: [row_count]
    gaps_field: gaps
    registry_records_field: datasets
    registry_key_fields: [dataset_id, version]
    registry_required_true_fields: [eligible]
    registry_status_field: status
    registry_allowed_statuses: [Healthy]
    registry_count_field: rows
    registry_identity_fields:
      instrument: symbol
      interval: timeframe
    payload_path_field: normalized_path
    payload_format: jsonl
    required_fields: [timestamp, value, measure]
    non_constant_fields: [value, measure]
    series_value_fields: [value, measure]
    series_overlap_min_rows: 3
    request_path_field: request_path
    requested_count_field: count
    response_path_field: response_path
    response_identity_fields:
      instrument: symbol
    validation_path_field: validation_path
    validation_status_field: status
    validation_checks_field: checks
    validation_check_status_field: status
    validation_failed_values: [failed]
    validation_passed_values: [passed]
```
"#,
    )
    .expect("task");

    let universe = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD task files at {}",
        tasks.display()
    ))
    .expect("extract")
    .expect("universe");
    let task = &universe.tasks[0];

    assert_eq!(task.canonical_task_id, "TASK-DEMO-017");
    assert_eq!(
        task.required_env_keys,
        vec!["PROJECT_TOKEN".to_string(), "TASK_TOKEN".to_string()]
    );
    assert_eq!(
        task.required_tools,
        vec!["fetch_demo".to_string(), "project_probe".to_string()]
    );
    assert_eq!(task.deliverable_contracts.len(), 1);
    let contract = &task.deliverable_contracts[0];
    assert!(contract.required_universe);
    assert_eq!(
        contract.instance_source_path.as_deref(),
        Some(".archon/demo/instances.json")
    );
    assert_eq!(contract.min_instances, 2);
    assert_eq!(contract.data_kind.as_deref(), Some("record_series"));
    assert_eq!(
        contract.non_constant_fields,
        vec!["value".to_string(), "measure".to_string()]
    );
    assert_eq!(contract.series_overlap_min_rows, 3);
    assert_eq!(
        contract
            .registry_identity_fields
            .get("instrument")
            .map(String::as_str),
        Some("symbol")
    );
    assert_eq!(
        contract.validation_failed_values,
        vec!["failed".to_string()]
    );
}

#[test]
fn runtime_workflow_code_contains_no_fixture_task_ids() {
    // D52/D75 gate: the generic workflow runtime must carry NO fixture ids,
    // fixture paths, or fixture-domain vocabulary. Ids/paths would break other
    // PRDs outright; domain vocabulary is how fixture assumptions quietly
    // fossilize into "generic" prompts and detectors.
    const FIXTURE_LITERALS: &[&str] = &["task-tdl", "trading-lab"];
    const DOMAIN_VOCABULARY: &[&str] = &[
        "backtest",
        "paper trading",
        "paper-trading",
        "paper_trading",
        "paper-readiness",
        "pine",
        "ohlcv",
        "polygon",
        "tradingview",
        "openbb",
    ];
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut runtime_sources = Vec::new();
    for entry in fs::read_dir(manifest_dir.join("src/command")).expect("read command sources") {
        let path = entry.expect("source entry").path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("workflow_live") && name.ends_with(".rs") && !name.contains("_tests") {
            runtime_sources.push(path);
        }
    }
    collect_workflow_crate_sources(
        &manifest_dir.join("crates/archon-workflow/src"),
        &mut runtime_sources,
    );
    assert!(
        !runtime_sources.is_empty(),
        "gate found no runtime sources to scan"
    );
    for path in runtime_sources {
        let source = fs::read_to_string(&path).expect("read runtime source");
        let runtime_only = source
            .split("\n#[cfg(test)]")
            .next()
            .unwrap_or(&source)
            .to_ascii_lowercase();
        for literal in FIXTURE_LITERALS {
            assert!(
                !runtime_only.contains(literal),
                "fixture literal '{literal}' leaked into runtime source {}",
                path.display()
            );
        }
        for word in DOMAIN_VOCABULARY {
            assert!(
                !runtime_only.contains(word),
                "fixture-domain vocabulary '{word}' leaked into runtime source {}",
                path.display()
            );
        }
    }
}

fn collect_workflow_crate_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.is_dir() {
            if !name.contains("fixture") && !name.contains("tests") {
                collect_workflow_crate_sources(&path, out);
            }
            continue;
        }
        if name.ends_with(".rs") && !name.contains("_tests") && name != "tests.rs" {
            out.push(path);
        }
    }
}
