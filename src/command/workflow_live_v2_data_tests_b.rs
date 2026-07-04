#[test]
fn item_producer_request_demands_flat_items_array() {
    let mut extra = BTreeMap::new();
    extra.insert("outputs".to_string(), serde_json::json!(["items"]));
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "discover".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions {
                extra,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::Value::Null,
        depends_on: Vec::new(),
    };

    let request = v2_agent_request("objective", Some("/repo".to_string()), &execution);

    assert!(
        request
            .constraints
            .iter()
            .any(|constraint| constraint.contains("data.items as a flat JSON array"))
    );
}

#[test]
fn reducer_source_pack_keeps_evidence_shape_but_bounds_large_text() {
    let long = "x".repeat(4_000);
    let packed = source_pack_value(&serde_json::json!({
        "status": "accepted",
        "summary": long,
        "evidence": [
            { "kind": "inspection", "summary": "read files" }
        ],
        "commands_run": [
            {
                "kind": "test",
                "command": "cargo test -p archon-workflow --test v2_runtime_sim",
                "status": "succeeded",
                "exit_code": 0,
                "output_summary": "y".repeat(3_000)
            }
        ],
        "files_changed": [
            { "path": "src/command/workflow_live_v2_script.rs", "purpose": "runtime repair" }
        ],
        "task_coverage": [
            {
                "task_id": "TASK-TDL-001",
                "status": "accepted",
                "summary": "implemented",
                "evidence": [{ "kind": "implementation", "summary": "changed files" }]
            }
        ],
        "residual_gaps": [],
        "data": {
            "items": [
                {
                    "status": "accepted",
                    "summary": "z".repeat(3_000),
                    "data": { "branch_artifact_paths": ["/tmp/branch.json"] }
                }
            ],
            "branch_artifact_paths": ["/tmp/branch.json"]
        }
    }));

    assert_eq!(packed["status"], "accepted");
    assert!(
        packed["summary"]
            .as_str()
            .is_some_and(|summary| summary.len() < 800 && summary.ends_with("..."))
    );
    assert_eq!(
        packed["commands_run"][0]["command"],
        "cargo test -p archon-workflow --test v2_runtime_sim"
    );
    assert_eq!(packed["evidence"][0]["summary"], "read files");
    assert!(
        packed["commands_run"][0]["output_summary"]
            .as_str()
            .is_some_and(|summary| summary.len() < 800 && summary.ends_with("..."))
    );
    assert_eq!(
        packed["files_changed"][0]["path"],
        "src/command/workflow_live_v2_script.rs"
    );
    assert_eq!(packed["task_coverage"][0]["task_id"], "TASK-TDL-001");
    assert_eq!(packed["branch_artifact_paths"][0], "/tmp/branch.json");
    assert!(
        packed["items"][0]["summary"]
            .as_str()
            .is_some_and(|summary| summary.len() < 800 && summary.ends_with("..."))
    );
}

#[test]
fn reducer_source_pack_preserves_bounded_outcomes_and_count() {
    let packed = source_pack_value(&serde_json::json!({
        "items": [],
        "outcomes": [
            { "item_id": "a", "transcript": "x".repeat(10_000) },
            { "item_id": "b", "transcript": "y".repeat(10_000) }
        ],
        "branch_artifact_paths": ["/tmp/a.json", "/tmp/b.json"]
    }));

    assert_eq!(packed["outcome_count"], 2);
    assert_eq!(packed["outcomes"][0]["item_id"], "a");
    assert!(packed["outcomes"][0].get("transcript").is_none());
    assert_eq!(packed["branch_artifact_paths"].as_array().unwrap().len(), 2);
}

#[test]
fn fanout_branch_inherits_target_files_from_inventory_item() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let mut inventory = WorkflowV2Result::accepted("inventory");
    inventory.data = serde_json::json!({
        "items": [
            {
                "id": "TDL-001",
                "target_files": ["src/lib.rs", "tests/lib.rs"]
            }
        ]
    });
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            "run",
            WorkflowV2HostCall {
                id: "inventory".to_string(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: WorkflowV2HostOptions::default(),
            },
            1,
            "input".to_string(),
            inventory,
            Vec::new(),
        ))
        .expect("save inventory");
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "implementationResults".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions {
                source: Some("inventory.items".to_string()),
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::Value::Null,
        depends_on: vec!["inventory".to_string()],
    };

    let branches = fanout_items_for_call(&execution, &store).expect("fanout items");

    assert_eq!(branches.len(), 1);
    assert_eq!(
        branches[0].call.options.target_files,
        vec!["src/lib.rs", "tests/lib.rs"]
    );
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "test".to_string(),
        task: "Implement".to_string(),
        target_repository_root: Some("/repo".to_string()),
        max_parallelism: 8,
        max_agents: 32,
        provider_tiers: BTreeMap::new(),
        stages: Vec::new(),
        artifact_policy: Default::default(),
        permissions: BTreeMap::new(),
        quality_gates: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let branch_execution = WorkflowV2CallExecution {
        call: branches[0].call.clone(),
        input: branches[0].input.clone(),
        depends_on: vec!["implementationResults".to_string()],
    };
    let request = v2_agent_request(
        "objective",
        spec.target_repository_root.clone(),
        &branch_execution,
    );

    assert_eq!(request.target_files, vec!["src/lib.rs", "tests/lib.rs"]);
}

#[test]
fn fanout_branch_item_targets_override_static_fallback_targets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let mut inventory = WorkflowV2Result::accepted("inventory");
    inventory.data = serde_json::json!({
        "items": [
            {
                "id": "TDL-001",
                "target_files": ["crates/archon-trading/src/data_lake.rs"]
            }
        ]
    });
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            "run",
            WorkflowV2HostCall {
                id: "inventory".to_string(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: WorkflowV2HostOptions::default(),
            },
            1,
            "input".to_string(),
            inventory,
            Vec::new(),
        ))
        .expect("save inventory");
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "implementationResults".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions {
                source: Some("inventory.items".to_string()),
                target_files: vec!["/repo".to_string()],
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::Value::Null,
        depends_on: vec!["inventory".to_string()],
    };

    let branches = fanout_items_for_call(&execution, &store).expect("fanout items");

    assert_eq!(
        branches[0].call.options.target_files,
        vec!["crates/archon-trading/src/data_lake.rs"]
    );
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "test".to_string(),
        task: "Implement".to_string(),
        target_repository_root: Some("/repo".to_string()),
        max_parallelism: 8,
        max_agents: 32,
        provider_tiers: BTreeMap::new(),
        stages: Vec::new(),
        artifact_policy: Default::default(),
        permissions: BTreeMap::new(),
        quality_gates: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let branch_execution = WorkflowV2CallExecution {
        call: branches[0].call.clone(),
        input: branches[0].input.clone(),
        depends_on: vec!["implementationResults".to_string()],
    };
    let request = v2_agent_request(
        "objective",
        spec.target_repository_root.clone(),
        &branch_execution,
    );

    assert_eq!(
        request.target_files,
        vec!["crates/archon-trading/src/data_lake.rs"]
    );
}
