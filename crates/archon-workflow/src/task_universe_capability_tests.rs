//! Declaration-driven capability merging.
//!
//! Split out of `task_universe_b.rs` only because each source file in this
//! tree is held under a 500-line ceiling; these are otherwise ordinary tests
//! of that module.
//!
//! The runtime-genericity gate that used to sit here stayed in the bin crate
//! as `workflow_live_runtime_genericity_tests.rs`. It scans both this crate's
//! sources and `src/command/workflow_live*`, and it locates them from
//! `CARGO_MANIFEST_DIR`, so it only resolves both halves from the crate that
//! is the workspace root.

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
        vec!["fetch_demo".to_string()],
        "the manifest's tools must not reach a task: a declared tool must be \
         exercised, so hoisting one obliges every task to invoke it"
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

/// #163 failure 3. A task that declares no tools must be handed none.
///
/// `required_tools` carries an invocation obligation: an accepted branch must
/// show a real invocation of every declared tool, and a task with any tool
/// declared may not declare a no-op. `sync-capabilities` used to hoist the
/// ambient toolchain into the manifest and this merge unioned it onto every
/// task, so a task with nothing to run under `python3` still had to run it —
/// accepted was unreachable and noop was forbidden. Environment keys, which are
/// proven rather than exercised, still merge: that half is the reason the
/// manifest exists.
#[test]
fn a_task_declaring_no_tools_is_not_given_the_projects_tools() {
    let project = tempfile::tempdir().expect("project");
    let archon = project.path().join(".archon");
    let tasks = project.path().join("tasks/PRD-DEMO");
    fs::create_dir_all(&archon).expect("archon dir");
    fs::create_dir_all(&tasks).expect("tasks dir");
    fs::write(
        archon.join("project.json"),
        serde_json::json!({
            "required_env_keys": ["PROJECT_TOKEN"],
            "required_tools": ["archon", "bash", "cargo", "python3"],
            "tool_bundles": { "lake": ["duckdb"] }
        })
        .to_string(),
    )
    .expect("project manifest");
    fs::write(
        tasks.join("TASK-DEMO-001-audit.md"),
        "# Audit\n\n```yaml\ntask_id: TASK-DEMO-001\ntitle: Audit\n\
         complexity: small\nstatus: ready\ndepends_on: []\nblocks: []\n\
         implements: []\nrequired_env_keys: []\nrequired_tools: []\n\
         deliverable_contracts: []\n```\n",
    )
    .expect("task");

    let universe = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD task files at {}",
        tasks.display()
    ))
    .expect("extract")
    .expect("universe");
    let task = &universe.tasks[0];

    assert!(
        task.required_tools.is_empty(),
        "a task that declares no tools must owe no invocations; got {:?}",
        task.required_tools
    );
    assert_eq!(
        task.required_env_keys,
        vec!["PROJECT_TOKEN".to_string()],
        "environment keys are proven, not exercised, and still merge"
    );
}
