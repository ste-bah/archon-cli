use super::*;

#[test]
fn task_universe_resolves_canonical_and_alias_forms() {
    let universe = synthetic_universe(&[
        ("TASK-ALPHA-010", &["T010"], &[]),
        ("TASK-ALPHA-020", &["T020"], &["TASK-ALPHA-010"]),
    ]);

    assert_eq!(
        universe
            .resolve_canonical_task_id("TASK-ALPHA-010")
            .unwrap(),
        "TASK-ALPHA-010"
    );
    assert_eq!(
        universe.resolve_canonical_task_id("ALPHA-010").unwrap(),
        "TASK-ALPHA-010"
    );
    assert_eq!(
        universe.resolve_canonical_task_id("T010").unwrap(),
        "TASK-ALPHA-010"
    );
}

#[test]
fn task_universe_rejects_ambiguous_short_aliases() {
    let universe = synthetic_universe(&[
        ("TASK-ALPHA-010", &["T010"], &[]),
        ("TASK-BETA-010", &["T010"], &[]),
    ]);

    let err = universe
        .resolve_canonical_task_id("T010")
        .expect_err("ambiguous short alias must fail");

    assert!(err.to_string().contains("ambiguous"));
    assert!(err.to_string().contains("TASK-ALPHA-010"));
    assert!(err.to_string().contains("TASK-BETA-010"));
}

#[test]
fn task_universe_computes_downstream_task_closure() {
    let universe = synthetic_universe(&[
        ("TASK-ALPHA-010", &["T010"], &[]),
        ("TASK-ALPHA-020", &["T020"], &["TASK-ALPHA-010"]),
        ("TASK-ALPHA-030", &["T030"], &["TASK-ALPHA-020"]),
        ("TASK-ALPHA-040", &["T040"], &[]),
    ]);

    assert_eq!(
        universe.downstream_task_closure("TASK-ALPHA-010"),
        ["TASK-ALPHA-010", "TASK-ALPHA-020", "TASK-ALPHA-030"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
}

#[test]
fn universe_comes_from_task_files_not_reducer_items() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: []\n",
    )
    .expect("task 1");
    fs::write(
        temp.path().join("TASK-TDL-010-dependent.md"),
        "# Dependent\n\ntask_id: TASK-TDL-010\ndepends_on: ['TASK-TDL-001']\n",
    )
    .expect("task 10");

    let universe = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect("extract")
    .expect("universe");

    assert_eq!(
        universe.canonical_ids(),
        vec!["TASK-TDL-001".to_string(), "TASK-TDL-010".to_string()]
    );
    assert_eq!(
        universe.tasks[1].dependency_ids,
        vec!["TASK-TDL-001".to_string()]
    );
}

#[test]
fn task_universe_carries_authoritative_acceptance_criteria() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: []\n\n## Acceptance Criteria\n\n- First exact criterion.\n- Second exact criterion with `literal` text.\n\n## Focused Tests\n\n- ignored test bullet\n",
    )
    .expect("task");

    let universe = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect("extract")
    .expect("universe");

    assert_eq!(
        universe.tasks[0].acceptance_criteria,
        vec![
            "First exact criterion.".to_string(),
            "Second exact criterion with `literal` text.".to_string(),
        ]
    );
}

/// `## Adversarial Review Notes` is the task author's own list of falsification
/// hypotheses. It only became reachable when review moved to one reviewer per
/// task — a reducer holding every task at once has no use for it. The first
/// file mirrors a real task file (TASK-TDL-001), including the trailing
/// prior-run-findings block that must not leak into the notes; the second
/// declares none, and gets none — nothing is invented for a reviewer to chase.
#[test]
fn task_universe_carries_declared_adversarial_review_notes() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: []\n\n## Acceptance Criteria\n\n- A criterion.\n\n## Adversarial Review Notes\n\n- Verify the task does not weaken native-candle enforcement.\n- Verify residual gaps fail closed.\n\n<!-- PRIOR-RUN-FINDINGS:BEGIN -->\n\n### Prior run `wf-ee4a92fc` (2026-07-28)\n\n- a prior finding bullet that is not a review note\n",
    )
    .expect("task");
    fs::write(
        temp.path().join("TASK-TDL-002-plain.md"),
        "# Plain\n\ntask_id: TASK-TDL-002\ndepends_on: []\n\n## Acceptance Criteria\n\n- A criterion.\n",
    )
    .expect("task");

    let universe = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect("extract")
    .expect("universe");

    assert_eq!(
        universe.tasks[0].adversarial_review_notes,
        vec![
            "Verify residual gaps fail closed.".to_string(),
            "Verify the task does not weaken native-candle enforcement.".to_string(),
        ]
    );
    assert!(universe.tasks[1].adversarial_review_notes.is_empty());
}

#[test]
fn prd_task_references_must_have_matching_task_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let prd = temp.path().join("PRD.md");
    fs::write(&prd, "Acceptance references TASK-TDL-140.\n").expect("prd");
    let tasks = temp.path().join("tasks");
    fs::create_dir_all(&tasks).expect("tasks");
    fs::write(
        tasks.join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: []\n",
    )
    .expect("task");

    let err = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {} and tasks at {}",
        prd.display(),
        tasks.display()
    ))
    .expect_err("unbacked PRD task reference must fail");

    assert!(err.to_string().contains("references TASK-TDL-140"));
}

#[test]
fn missing_authoritative_task_evidence_fails_for_decomposed_prd() {
    let err = extract_task_universe_for_generated_run(
        "Implement the decomposed PRD at /no/such/tasks/PRD-MISSING",
    )
    .expect_err("missing local evidence must fail");

    assert!(
        err.to_string()
            .contains("requires local TASK-*.md evidence")
    );
}

#[test]
fn invalid_task_id_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-1\ndepends_on: []\n",
    )
    .expect("task");

    let err = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect_err("invalid task id must fail");

    assert!(err.to_string().contains("invalid task_id"));
}

#[test]
fn dependency_cycles_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: [TASK-TDL-010]\n",
    )
    .expect("task 1");
    fs::write(
        temp.path().join("TASK-TDL-010-dependent.md"),
        "# Dependent\n\ntask_id: TASK-TDL-010\ndepends_on: [TASK-TDL-001]\n",
    )
    .expect("task 10");

    let err = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect_err("cycle must fail");

    assert!(err.to_string().contains("dependency cycle"));
}

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
depends_on: []
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
    assert_eq!(
        contract.instance_source_records_field.as_deref(),
        Some("records")
    );
    assert_eq!(
        contract.instance_artifact_field.as_deref(),
        Some("report_path")
    );
    assert_eq!(contract.min_instances, 2);
    assert_eq!(contract.data_kind.as_deref(), Some("record_series"));
    assert_eq!(
        contract.universe_fields,
        vec!["instruments".to_string(), "intervals".to_string()]
    );
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
        contract
            .response_identity_fields
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

#[test]
fn malformed_deliverable_contract_fails_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-DEMO-017-invalid.md"),
        r#"```yaml
task_id: TASK-DEMO-017
depends_on: []
deliverable_contracts:
  - kind: required_universe_registry
    artifact_path: 42
```"#,
    )
    .expect("task");

    let error = extract_task_universe_for_generated_run(&format!(
        "Implement decomposed PRD task files at {}",
        temp.path().display()
    ))
    .expect_err("invalid capability contract must fail");

    assert!(error.to_string().contains("invalid type"));
}

fn synthetic_universe(tasks: &[(&str, &[&str], &[&str])]) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["tasks".to_string()],
        tasks: tasks
            .iter()
            .map(
                |(canonical_task_id, aliases, dependency_ids)| WorkflowV2TaskUniverseTask {
                    canonical_task_id: (*canonical_task_id).to_string(),
                    aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
                    source_path: format!("tasks/{canonical_task_id}.md"),
                    dependency_ids: dependency_ids
                        .iter()
                        .map(|dependency| (*dependency).to_string())
                        .collect(),
                    title: None,
                    artifact_requirements: Vec::new(),
                    ..Default::default()
                },
            )
            .collect(),
    }
}
