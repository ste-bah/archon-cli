use super::*;

#[test]
fn dynamic_wave_source_hash_is_stable_across_ready_item_order() {
    let left = execution(serde_json::json!([
        {
            "id": "alpha",
            "canonical_task_ids": ["TASK-TDL-001"],
            "dependency_ids": [],
            "target_files": ["src/a.rs"]
        },
        {
            "id": "beta",
            "canonical_task_ids": ["TASK-TDL-010"],
            "dependency_ids": ["alpha"],
            "target_files": ["src/b.rs"]
        }
    ]));
    let right = execution(serde_json::json!([
        {
            "target_files": ["src/b.rs"],
            "dependency_ids": ["alpha"],
            "canonical_task_ids": ["TASK-TDL-010"],
            "id": "beta"
        },
        {
            "target_files": ["src/a.rs"],
            "dependency_ids": [],
            "canonical_task_ids": ["TASK-TDL-001"],
            "id": "alpha"
        }
    ]));

    let universe = task_universe();
    let left = dynamic_wave_source_metadata(&left, Some(&universe), None);
    let right = dynamic_wave_source_metadata(&right, Some(&universe), None);

    assert_eq!(left.unresolved_dependencies, Vec::<String>::new());
    assert_eq!(left.source_fingerprint, right.source_fingerprint);
    assert_eq!(
        left.source_task_graph.unwrap().items[1].dependency_ids,
        vec!["TASK-TDL-001"]
    );
}

#[test]
fn dynamic_wave_source_normalizes_short_dependency_ids_against_task_universe() {
    let execution = execution(serde_json::json!([
        {
            "id": "beta",
            "canonical_task_ids": ["TASK-TDL-010"],
            "dependency_ids": ["T001"],
            "target_files": ["src/b.rs"]
        }
    ]));

    let universe = task_universe();
    let metadata = dynamic_wave_source_metadata(&execution, Some(&universe), None);
    let graph = metadata.source_task_graph.expect("graph");

    assert_eq!(metadata.unresolved_dependencies, Vec::<String>::new());
    assert_eq!(graph.items[0].dependency_ids, vec!["TASK-TDL-001"]);
    assert!(metadata.source_fingerprint.is_some());
}

#[test]
fn noop_source_metadata_accepts_proof_references_alias() {
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "noop-proof-verification-1".to_string(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options: WorkflowV2HostOptions {
                item_kind: Some("noop_proof".to_string()),
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({
            "objective": "Verify no-op proof",
            "source_data": [
                {
                    "id": "noop-foundation",
                    "canonical_task_id": "T001",
                    "dependencies": [],
                    "work_type": "verified_noop",
                    "acceptanceCriteria": ["foundation accepted"],
                    "noop_proof": "already implemented",
                    "proof_references": ["src/lib.rs:10"]
                }
            ],
        }),
        depends_on: Vec::new(),
    };

    let universe = task_universe();
    let metadata = dynamic_wave_source_metadata(&execution, Some(&universe), None);

    assert_eq!(metadata.unresolved_dependencies, Vec::<String>::new());
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("source graph");
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-001"]);
}

#[test]
fn remediation_source_metadata_requires_repair_graph_fields_but_allows_empty_dependencies() {
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "remediation-wave-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".to_string()),
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({
            "objective": "Remediate decomposed PRD",
            "source_data": [{
                "id": "remediate-010",
                "source_item_id": "impl-010",
                "canonical_task_ids": ["TASK-TDL-010"],
                "dependency_ids": [],
                "target_files": ["src/lib.rs"],
                "focused_verification": ["cargo test focused"],
                "artifact_requirements": [],
                "failure_status": "needs_review",
                "failure_evidence": ["missing command proof"],
                "required_fix": "add focused verification evidence",
                "verification_requirements": ["cargo test focused"]
            }],
        }),
        depends_on: Vec::new(),
    };

    let metadata = dynamic_wave_source_metadata(&execution, Some(&task_universe()), None);

    assert_eq!(metadata.unresolved_dependencies, Vec::<String>::new());
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-010"]);
    assert_eq!(graph.items[0].dependency_ids, Vec::<String>::new());
}

#[test]
fn remediation_source_metadata_accepts_failure_kind_as_failure_evidence() {
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "remediation-wave-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".to_string()),
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({
            "source_data": [{
                "id": "remediate-010-followup",
                "source_item_id": "remediate-010",
                "canonical_task_ids": ["TASK-TDL-010"],
                "dependency_ids": [],
                "target_files": ["src/lib.rs"],
                "focused_verification": ["cargo test focused"],
                "artifact_requirements": [],
                "status": "needs_review",
                "failure_kind": "verification_failed",
                "required_fix": "repair failed focused verification",
                "verification_requirements": ["cargo test focused"]
            }],
        }),
        depends_on: Vec::new(),
    };

    let metadata = dynamic_wave_source_metadata(&execution, Some(&task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-010"]);
}

#[test]
fn remediation_source_metadata_treats_task_source_root_paths_as_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let tasks = temp.path().join("project/tasks");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    std::fs::create_dir_all(&tasks).expect("tasks");
    std::fs::write(repo.join("src/lib.rs"), "").expect("source");
    let task_doc = tasks.join("TASK-TDL-010.md");
    std::fs::write(&task_doc, "# task").expect("task doc");
    let mut universe = task_universe();
    universe.source_roots = vec![tasks.display().to_string()];
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "remediation-wave-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".to_string()),
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({
            "objective": "Remediate decomposed PRD",
            "source_data": [{
                "id": "remediate-010",
                "source_item_id": "impl-010",
                "canonical_task_ids": ["TASK-TDL-010"],
                "dependency_ids": [],
                "target_files": [task_doc.display().to_string(), "src/lib.rs"],
                "focused_verification": ["inspect task evidence"],
                "artifact_requirements": [],
                "failure_status": "needs_review",
                "failure_evidence": ["missing proof"],
                "required_fix": "repair evidence",
                "verification_requirements": ["inspect task evidence"]
            }],
        }),
        depends_on: Vec::new(),
    };

    let metadata = dynamic_wave_source_metadata(
        &execution,
        Some(&universe),
        Some(&repo.display().to_string()),
    );

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    let item = &metadata.source_task_graph.expect("graph").items[0];
    assert_eq!(item.target_files, vec!["src/lib.rs"]);
    assert!(
        item.artifact_requirements
            .contains(&task_doc.display().to_string())
    );
}

#[test]
fn review_remediation_source_metadata_accepts_wf580_fixture_shape() {
    let fixture = include_str!("fixtures/wf580_review_remediation_inventory_items.json");
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized_items = source_data
        .as_array()
        .expect("fixture array")
        .iter()
        .map(|value| {
            crate::command::workflow_live::workflow_live_generated_contract::normalize_generated_item_value(
                value,
                Some(&tdl_task_universe()),
            )
            .value
        })
        .collect::<Vec<_>>();
    for value in &normalized_items {
        assert!(
            review_remediation_item_has_required_fields(value),
            "normalized review remediation item lacks required graph fields: {value:#}"
        );
    }
    let execution = review_remediation_execution(source_data);

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.unresolved_dependencies, Vec::<String>::new());
    assert_eq!(metadata.invalid_reason, None);
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items.len(), 2);
    assert_eq!(graph.items[0].item_id, "REM-TDL-010");
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-010"]);
    assert_eq!(graph.items[0].dependency_ids, vec!["TASK-TDL-001"]);
    assert_eq!(graph.items[1].dependency_ids, vec!["TASK-TDL-010"]);
}

#[test]
fn review_verification_source_metadata_accepts_wf580_fixture_shape() {
    let fixture = include_str!("fixtures/wf580_review_verification_plan_items.json");
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let execution = review_verification_execution(source_data);

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.unresolved_dependencies, Vec::<String>::new());
    assert_eq!(metadata.invalid_reason, None);
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items.len(), 2);
    assert_eq!(graph.items[0].item_id, "VERIFY-TDL-010");
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-010"]);
    assert_eq!(graph.items[1].dependency_ids, vec!["TASK-TDL-010"]);
    assert!(
        graph.items[0]
            .artifact_requirements
            .iter()
            .any(|path| path == ".archon/trading-lab/data/registry.json")
    );
}

#[test]
fn focused_verification_allows_multiple_checks_for_one_canonical_task() {
    let fixture = include_str!("fixtures/wf139e_verification_plan_items.json");
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let execution = focused_verification_execution(source_data);

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.unresolved_dependencies, Vec::<String>::new());
    assert_eq!(metadata.invalid_reason, None);
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items.len(), 4);
    assert!(
        graph
            .items
            .iter()
            .all(|item| item.canonical_task_ids == vec!["TASK-TDL-010"])
    );
    assert!(graph.items.iter().any(|item| {
        item.focused_verification
            .iter()
            .any(|value| value.contains("cargo test -p archon-trading registry"))
    }));
    assert!(graph.items.iter().any(|item| {
        item.artifact_requirements
            .iter()
            .any(|value| value.contains("registry.json"))
    }));
}

#[test]
fn focused_verification_repair_fixtures_produce_reusable_source_metadata() {
    for fixture in [
        include_str!("fixtures/wf139e_verification_repair_plan_1_1_items.json"),
        include_str!("fixtures/wf139e_verification_repair_plan_1_2_items.json"),
        include_str!("fixtures/wf139e_verification_repair_plan_1_3_items.json"),
    ] {
        let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
        let execution = focused_verification_execution(source_data);

        let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

        assert_eq!(metadata.unresolved_dependencies, Vec::<String>::new());
        assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
        assert!(metadata.source_fingerprint.is_some());
        let graph = metadata.source_task_graph.expect("graph");
        assert!(graph.items.iter().all(|item| !item.item_id.is_empty()));
        assert!(graph.items.iter().all(|item| {
            item.canonical_task_ids
                .contains(&"TASK-TDL-010".to_string())
        }));
        assert!(
            graph
                .items
                .iter()
                .all(|item| !item.focused_verification.is_empty())
        );
        assert!(graph.items.iter().all(|item| {
            item.artifact_requirements
                .iter()
                .all(|value| !value.is_empty())
        }));
    }
}
