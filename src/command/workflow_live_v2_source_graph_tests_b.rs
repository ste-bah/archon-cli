#[test]
fn implementation_source_graph_expands_declared_rust_module_targets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src/data_store")).expect("module dir");
    std::fs::write(
        repo.join("crates/archon-trading/src/data_store.rs"),
        "mod io;\npub mod migration;\nmod inline {}\nmod missing;\n",
    )
    .expect("data_store");
    std::fs::write(repo.join("crates/archon-trading/src/data_store/io.rs"), "").expect("io");
    std::fs::write(
        repo.join("crates/archon-trading/src/data_store/migration.rs"),
        "",
    )
    .expect("migration");
    let execution = execution(serde_json::json!([
        {
            "id": "inventory-tdl-010-registry-schema-v2",
            "canonical_task_ids": ["TASK-TDL-010"],
            "dependency_ids": ["TASK-TDL-001"],
            "target_files": ["crates/archon-trading/src/data_store.rs"]
        }
    ]));

    let metadata = dynamic_wave_source_metadata(
        &execution,
        Some(&task_universe()),
        Some(&repo.display().to_string()),
    );

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    let item = &graph.items[0];
    assert_eq!(
        item.declared_target_files,
        vec!["crates/archon-trading/src/data_store.rs"]
    );
    assert!(item
        .target_files
        .contains(&"crates/archon-trading/src/data_store/io.rs".to_string()));
    assert!(item
        .target_files
        .contains(&"crates/archon-trading/src/data_store/migration.rs".to_string()));
    assert_eq!(
        item.target_file_expansions[0].source,
        "crates/archon-trading/src/data_store.rs"
    );
    assert!(item.target_file_expansions[0]
        .notes
        .iter()
        .any(|note| note.contains("missing")));
}

#[test]
fn focused_verification_rejects_missing_verification_evidence_fields() {
    let execution = focused_verification_execution(serde_json::json!([
        {
            "id": "verify-TASK-TDL-010-missing-evidence",
            "canonical_task_ids": ["TASK-TDL-010"],
            "artifact_requirements": []
        }
    ]));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert!(metadata.source_metadata_required);
    assert_eq!(metadata.source_fingerprint, None);
    assert!(
        metadata
            .invalid_reason
            .as_deref()
            .unwrap_or_default()
            .contains("focused verification source items must include"),
        "{metadata:?}"
    );
}

#[test]
fn focused_verification_accepts_retry_expected_evidence() {
    let execution = focused_verification_execution(serde_json::json!([
        {
            "id": "retry-verify-TASK-TDL-010-focused-compile-check",
            "canonical_task_ids": ["TASK-TDL-010"],
            "commands": ["cargo check -p archon-trading --lib"],
            "expected_evidence": "cargo check exits 0 with no compiler errors"
        }
    ]));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-010"]);
    assert_eq!(
        graph.items[0].focused_verification,
        vec!["cargo check -p archon-trading --lib"]
    );
    assert_eq!(
        graph.items[0].expected_evidence,
        vec!["cargo check exits 0 with no compiler errors"]
    );
    assert!(graph.items[0].artifact_requirements.is_empty());
}

#[test]
fn focused_verification_expected_evidence_changes_source_fingerprint() {
    let source = |expected: &str| {
        focused_verification_execution(serde_json::json!([
            {
                "id": "retry-verify-TASK-TDL-010-focused-compile-check",
                "canonical_task_ids": ["TASK-TDL-010"],
                "commands": ["cargo check -p archon-trading --lib"],
                "expected_evidence": expected
            }
        ]))
    };

    let left = dynamic_wave_source_metadata(
        &source("cargo check exits 0"),
        Some(&tdl_task_universe()),
        None,
    );
    let right = dynamic_wave_source_metadata(
        &source("cargo check exits 0 and emits no warnings"),
        Some(&tdl_task_universe()),
        None,
    );

    assert_ne!(left.source_fingerprint, right.source_fingerprint);
}

#[test]
fn focused_verification_accepts_wffe12_retry_steps_fixture() {
    let fixture = include_str!("fixtures/wffe12_verification_repair_plan_1_3_items.json");
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let execution = focused_verification_execution(source_data);

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items.len(), 2);
    assert!(graph.items.iter().all(|item| {
        item.canonical_task_ids == vec!["TASK-TDL-050", "TASK-TDL-070"]
            && !item.focused_verification.is_empty()
            && !item.expected_evidence.is_empty()
            && item.artifact_requirements.is_empty()
    }));
}

#[test]
fn focused_verification_accepts_nested_retry_repair_plan_fixture() {
    let fixture = include_str!("fixtures/wff68_verification_repair_plan_1_1.json");
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized =
        crate::command::workflow_live::workflow_live_generated_contract::normalize_generated_inventory_value(
            &source_data,
            Some(&tdl_task_universe()),
        );
    let execution = focused_verification_execution(serde_json::Value::Array(normalized.items));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items.len(), 3);
    assert!(graph.items.iter().all(|item| {
        item.canonical_task_ids == vec!["TASK-TDL-001"]
            && !item.focused_verification.is_empty()
            && !item.expected_evidence.is_empty()
    }));
}

#[test]
fn focused_verification_accepts_direct_retry_items_fixture() {
    let fixture = include_str!("fixtures/wf1ca_verification_repair_plan_1_1.json");
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized =
        crate::command::workflow_live::workflow_live_generated_contract::normalize_generated_inventory_value(
            &source_data,
            Some(&tdl_task_universe()),
        );
    let execution = focused_verification_execution(serde_json::Value::Array(normalized.items));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items.len(), 1);
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-001"]);
    assert!(!graph.items[0].focused_verification.is_empty());
    assert!(!graph.items[0].expected_evidence.is_empty());
}

#[test]
fn focused_verification_accepts_retry_command_fixture() {
    let fixture = include_str!("fixtures/wf19f5_verification_repair_plan_1_1.json");
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized =
        crate::command::workflow_live::workflow_live_generated_contract::normalize_generated_inventory_value(
            &source_data,
            Some(&tdl_task_universe()),
        );
    let execution = focused_verification_execution(serde_json::Value::Array(normalized.items));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items.len(), 2);
    assert!(graph.items.iter().all(|item| {
        item.canonical_task_ids == vec!["TASK-TDL-010"]
            && !item.focused_verification.is_empty()
            && !item.expected_evidence.is_empty()
    }));
}

#[test]
fn focused_verification_accepts_nested_result_retry_fixture() {
    let fixture = include_str!("fixtures/wf19f5_verification_repair_plan_1_3.json");
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized =
        crate::command::workflow_live::workflow_live_generated_contract::normalize_generated_inventory_value(
            &source_data,
            Some(&tdl_task_universe()),
        );
    let execution = focused_verification_execution(serde_json::Value::Array(normalized.items));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items.len(), 1);
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-010"]);
    assert!(!graph.items[0].focused_verification.is_empty());
    assert!(!graph.items[0].expected_evidence.is_empty());
}

#[test]
fn dynamic_wave_source_blocks_unresolved_dependency_reuse() {
    let execution = execution(serde_json::json!([
        {
            "id": "beta",
            "canonical_task_ids": ["TASK-TDL-010"],
            "dependency_ids": ["setup-provider"],
            "target_files": ["src/b.rs"]
        }
    ]));

    let universe = task_universe();
    let metadata = dynamic_wave_source_metadata(&execution, Some(&universe), None);

    assert_eq!(
        metadata.unresolved_dependencies,
        vec!["setup-provider".to_string()]
    );
    assert!(metadata.source_metadata_required);
    assert!(metadata.source_fingerprint.is_none());
}

#[test]
fn dynamic_wave_source_rejects_self_dependency_graph_without_fingerprint() {
    let execution = execution(serde_json::json!([
        {
            "id": "bad-010",
            "canonical_task_ids": ["TASK-TDL-010"],
            "dependency_ids": ["TASK-TDL-010"],
            "target_files": ["src/b.rs"]
        }
    ]));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&task_universe()), None);

    assert!(metadata.source_metadata_required);
    assert!(metadata.source_fingerprint.is_none());
    assert!(metadata.unresolved_dependencies.is_empty());
    assert!(
        metadata
            .invalid_reason
            .as_deref()
            .unwrap_or_default()
            .contains("also claims"),
        "{metadata:?}"
    );
}

#[test]
fn dynamic_wave_source_rejects_duplicate_task_assignment_without_fingerprint() {
    let execution = execution(serde_json::json!([
        {
            "id": "left-010",
            "canonical_task_ids": ["TASK-TDL-010"],
            "dependency_ids": ["TASK-TDL-001"],
            "target_files": ["src/left.rs"]
        },
        {
            "id": "right-010",
            "canonical_task_ids": ["TASK-TDL-010"],
            "dependency_ids": ["TASK-TDL-001"],
            "target_files": ["src/right.rs"]
        }
    ]));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&task_universe()), None);

    assert!(metadata.source_metadata_required);
    assert!(metadata.source_fingerprint.is_none());
    assert!(
        metadata
            .invalid_reason
            .as_deref()
            .unwrap_or_default()
            .contains("assigned by multiple source items"),
        "{metadata:?}"
    );
}

#[test]
fn dynamic_wave_source_without_authoritative_universe_is_not_reusable() {
    let execution = execution(serde_json::json!([
        {
            "id": "alpha",
            "canonical_task_ids": ["TASK-TDL-001"],
            "dependency_ids": [],
            "target_files": ["src/a.rs"]
        }
    ]));

    let metadata = dynamic_wave_source_metadata(&execution, None, None);

    assert!(metadata.source_metadata_required);
    assert!(metadata.source_fingerprint.is_none());
    assert!(metadata.source_task_graph.is_none());
}
