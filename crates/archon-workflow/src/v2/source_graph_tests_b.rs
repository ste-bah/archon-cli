use super::*;

#[test]
fn required_tools_come_from_the_task_universe_not_the_agent_item() {
    // The item forges required_tools/mcp_tools, but the authoritative task
    // declares a different set. The graph item must carry ONLY the universe's
    // tools — an agent cannot bind MCP tools by injecting a declaration.
    use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-120".to_string(),
            aliases: vec!["T120".to_string()],
            source_path: "/tmp/tasks/TASK-TDL-120.md".to_string(),
            required_tools: vec!["pine_compile".to_string()],
            ..Default::default()
        }],
    };
    let execution = execution(serde_json::json!([
        {
            "id": "impl-tdl-120",
            "canonical_task_ids": ["TASK-TDL-120"],
            "target_files": ["src/lib.rs"],
            "required_tools": ["forged_tool"],
            "mcp_tools": ["another_forged"]
        }
    ]));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&universe), None);
    let graph = metadata.source_task_graph.expect("graph");

    assert_eq!(
        graph.items[0].required_tools,
        vec!["pine_compile".to_string()]
    );
}

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
    assert!(
        item.target_files
            .contains(&"crates/archon-trading/src/data_store/io.rs".to_string())
    );
    assert!(
        item.target_files
            .contains(&"crates/archon-trading/src/data_store/migration.rs".to_string())
    );
    assert_eq!(
        item.target_file_expansions[0].source,
        "crates/archon-trading/src/data_store.rs"
    );
    assert!(
        item.target_file_expansions[0]
            .notes
            .iter()
            .any(|note| note.contains("missing"))
    );
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
            .contains(
                "focused verification source_data[0].focused_verification is missing or empty"
            ),
        "{metadata:?}"
    );
}

#[test]
fn goal_oriented_verifier_item_without_pinned_commands_is_valid() {
    // The v3 prelude's verify:true item when the author pins no commands:
    // the prompt rides as verification_requirements (what to prove), the
    // agent chooses its own commands in-session. Run-9 blocked because this
    // shape was rejected; it must satisfy the wave metadata contract.
    let execution = focused_verification_execution(serde_json::json!([
        {
            "item_id": "verify-task-tdl-010-check",
            "canonical_task_ids": ["TASK-TDL-010"],
            "task": "You did NOT implement TASK-TDL-010. Re-read the task file and run whatever tests you judge decisive.",
            "instructions": "You did NOT implement TASK-TDL-010. Re-read the task file and run whatever tests you judge decisive.",
            "focused_verification": [],
            "artifact_requirements": [],
            "verification_requirements": ["You did NOT implement TASK-TDL-010. Re-read the task file and run whatever tests you judge decisive."]
        }
    ]));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_task_graph.is_some());
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
fn focused_verification_rejects_retry_steps_without_invariants() {
    let fixture = archon_test_support::fixtures::WFFE12_VERIFICATION_REPAIR_PLAN_1_3_ITEMS;
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let execution = focused_verification_execution(source_data);

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert!(metadata.invalid_reason.is_some(), "{metadata:?}");
    assert!(metadata.source_fingerprint.is_none());
}

#[test]
fn focused_verification_accepts_nested_retry_repair_plan_fixture() {
    let fixture = archon_test_support::fixtures::WFF68_VERIFICATION_REPAIR_PLAN_1_1;
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized = crate::generated_contract::normalize_generated_inventory_value(
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
fn focused_verification_rejects_direct_retry_items_without_invariants() {
    let fixture = archon_test_support::fixtures::WF1CA_VERIFICATION_REPAIR_PLAN_1_1;
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized = crate::generated_contract::normalize_generated_inventory_value(
        &source_data,
        Some(&tdl_task_universe()),
    );
    let execution = focused_verification_execution(serde_json::Value::Array(normalized.items));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert!(metadata.invalid_reason.is_some(), "{metadata:?}");
    assert!(metadata.source_fingerprint.is_none());
}

#[test]
fn focused_verification_accepts_retry_command_fixture() {
    let fixture = archon_test_support::fixtures::WF19F5_VERIFICATION_REPAIR_PLAN_1_1;
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized = crate::generated_contract::normalize_generated_inventory_value(
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
fn focused_verification_rejects_nested_retry_without_invariants() {
    let fixture = archon_test_support::fixtures::WF19F5_VERIFICATION_REPAIR_PLAN_1_3;
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized = crate::generated_contract::normalize_generated_inventory_value(
        &source_data,
        Some(&tdl_task_universe()),
    );
    let execution = focused_verification_execution(serde_json::Value::Array(normalized.items));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert!(metadata.invalid_reason.is_some(), "{metadata:?}");
    assert!(metadata.source_fingerprint.is_none());
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
fn d43_verification_remediation_allows_multiple_repairs_for_one_canonical_task() {
    let fixture: serde_json::Value =
        serde_json::from_str(archon_test_support::fixtures::D43_SAME_TASK_VERIFICATION_REMEDIATION)
            .expect("D43 fixture");
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "remediation-wave-10-verification-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".to_string()),
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({ "source_data": fixture["source_data"] }),
        depends_on: Vec::new(),
    };
    let mut universe = tdl_task_universe();
    for task_id in ["TASK-TDL-090", "TASK-TDL-110", "TASK-TDL-120"] {
        universe
            .tasks
            .push(crate::task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: task_id.to_string(),
                aliases: Vec::new(),
                source_path: format!("/tmp/tasks/{task_id}.md"),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            });
    }
    universe
        .tasks
        .push(crate::task_universe::WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-TDL-130".to_string(),
            aliases: Vec::new(),
            source_path: "/tmp/tasks/TASK-TDL-130.md".to_string(),
            dependency_ids: vec![
                "TASK-TDL-090".to_string(),
                "TASK-TDL-110".to_string(),
                "TASK-TDL-120".to_string(),
            ],
            title: None,
            artifact_requirements: Vec::new(),
            ..Default::default()
        });

    let metadata = dynamic_wave_source_metadata(&execution, Some(&universe), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    assert!(metadata.source_fingerprint.is_some());
    let graph = metadata.source_task_graph.expect("source graph");
    assert_eq!(graph.items.len(), 2);
    assert!(
        graph
            .items
            .iter()
            .all(|item| item.canonical_task_ids == ["TASK-TDL-130"])
    );
    assert_eq!(
        fixture["rejected"]["data"]["source_metadata_invalid"],
        "source graph canonical task 'TASK-TDL-130' is assigned by multiple source items"
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
