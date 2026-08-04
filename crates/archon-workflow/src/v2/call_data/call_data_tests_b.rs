use super::*;

#[test]
fn completion_claim_request_gets_authoritative_universe_without_mutating_execution() {
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "completion-claim-repair-2".to_string(),
            method: WorkflowV2HostMethod::Reduce,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        input: serde_json::json!([{"item_id":"claim-1"}]),
        depends_on: Vec::new(),
    };
    let original = execution.input.clone();
    let original_hash = serde_json::to_string(&execution.input).unwrap();
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["project-tasks".to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-1".to_string(),
            acceptance_criteria: vec!["completion acceptance detail".to_string()],
            deliverable_contracts: vec![WorkflowV2DeliverableContract {
                kind: "json".to_string(),
                artifact_path: "claim.json".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }],
    };

    let request = v2_agent_request(
        "objective",
        Some("/repo".to_string()),
        &execution,
        Some(&universe),
    );

    assert_eq!(execution.input, original);
    assert_eq!(
        serde_json::to_string(&execution.input).unwrap(),
        original_hash
    );
    assert!(
        request
            .input
            .to_string()
            .contains("completion acceptance detail")
    );
    assert!(request.input.to_string().contains("deliverable_contracts"));
    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);
    assert!(prompt.invocation.contains("completion acceptance detail"));
    assert!(prompt.invocation.contains("deliverable_contracts"));
}

#[test]
fn completion_claim_transport_retry_gets_authoritative_universe_once() {
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "completion-claim-repair-2-transport-retry-3".to_string(),
            method: WorkflowV2HostMethod::Reduce,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        input: serde_json::json!([{"item_id":"claim-1"}]),
        depends_on: Vec::new(),
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["project-tasks".to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-1".to_string(),
            acceptance_criteria: vec!["retry acceptance detail".to_string()],
            ..Default::default()
        }],
    };

    let request = v2_agent_request("objective", None, &execution, Some(&universe));
    let serialized = request.input.to_string();

    assert_eq!(
        serialized.matches("workflow-v2-task-universe-v1").count(),
        1
    );
    assert!(serialized.contains("retry acceptance detail"));
}

#[test]
fn completion_claim_decoy_universe_does_not_suppress_authoritative_universe() {
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "completion-claim-repair-3".to_string(),
            method: WorkflowV2HostMethod::Reduce,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        input: serde_json::json!([{"taskUniverse":{
            "schema_version":"decoy",
            "source_roots":[],
            "tasks":[],
            "label":"decoy metadata"
        }}]),
        depends_on: Vec::new(),
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["project-tasks".to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-1".to_string(),
            acceptance_criteria: vec!["authoritative acceptance".to_string()],
            ..Default::default()
        }],
    };

    let request = v2_agent_request("objective", None, &execution, Some(&universe));
    let prompt = WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    assert!(prompt.invocation.contains("decoy metadata"));
    assert!(prompt.invocation.contains("authoritative acceptance"));
}

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

    let request = v2_agent_request("objective", Some("/repo".to_string()), &execution, None);

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
        schema: crate::spec::WORKFLOW_SCHEMA.to_string(),
        name: "test".to_string(),
        task: "Implement".to_string(),
        target_repository_root: Some("/repo".to_string()),
        max_parallelism: 8,
        max_agents: 32,
        stages: Vec::new(),
        permissions: BTreeMap::new(),
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
        None,
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
        schema: crate::spec::WORKFLOW_SCHEMA.to_string(),
        name: "test".to_string(),
        task: "Implement".to_string(),
        target_repository_root: Some("/repo".to_string()),
        max_parallelism: 8,
        max_agents: 32,
        stages: Vec::new(),
        permissions: BTreeMap::new(),
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
        None,
    );

    assert_eq!(
        request.target_files,
        vec!["crates/archon-trading/src/data_lake.rs"]
    );
}
