#[test]
fn focused_verification_accepts_retry_plan_fixture() {
    let fixture = include_str!("fixtures/wf0eca_verification_repair_plan_1_1_item.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized =
        crate::command::workflow_live::workflow_live_generated_contract::normalize_generated_item_value(
            &value,
            Some(&tdl_task_universe()),
        )
        .value;
    let execution = focused_verification_execution(serde_json::json!([normalized]));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-010"]);
    assert!(!graph.items[0].focused_verification.is_empty());
    assert!(!graph.items[0].expected_evidence.is_empty());
}

#[test]
fn focused_verification_accepts_required_evidence_retry_commands_fixture() {
    let fixture = include_str!("fixtures/wf0eca_verification_repair_plan_1_2_item.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let normalized =
        crate::command::workflow_live::workflow_live_generated_contract::normalize_generated_item_value(
            &value,
            Some(&tdl_task_universe()),
        )
        .value;
    let execution = focused_verification_execution(serde_json::json!([normalized]));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-010"]);
    assert_eq!(
        graph.items[0].focused_verification,
        vec!["cargo test trading_data_prd_commands_parse"]
    );
    assert!(!graph.items[0].expected_evidence.is_empty());
}

#[test]
fn focused_verification_accepts_lowercase_embedded_retry_item_ids() {
    let fixture = include_str!("fixtures/wf199_verification_repair_plan_1_1.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let source_data = value["data"]["items"].clone();
    let execution = focused_verification_execution(source_data);

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items.len(), 3);
    assert!(graph.items.iter().all(|item| {
        item.canonical_task_ids == vec!["TASK-TDL-001"]
            && !item.focused_verification.is_empty()
            && !item.expected_evidence.is_empty()
    }));
}

#[test]
fn focused_verification_accepts_direct_command_retry_fixture() {
    let fixture = include_str!("fixtures/wfc5d4_verification_repair_plan_1_3.json");
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
        item.canonical_task_ids == vec!["TASK-TDL-010"]
            && !item.focused_verification.is_empty()
            && !item.expected_evidence.is_empty()
    }));
}

#[test]
fn review_remediation_accepts_artifact_only_canonical_task_item() {
    let item = serde_json::json!({
        "item_id": "remediate-project-artifact",
        "canonical_task_ids": ["TASK-TDL-050"],
        "dependency_ids": [],
        "source_item_id": "review-gap-1",
        "failure_status": "missing_project_artifact",
        "failure_evidence": ["required project artifact is absent"],
        "required_fix": "create the missing project artifact under the runtime project root",
        "target_files": [],
        "focused_verification": ["verify the project artifact exists and matches schema"],
        "artifact_requirements": ["project .archon data artifact"]
    });
    let execution = review_remediation_execution(serde_json::json!([item]));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items[0].canonical_task_ids, vec!["TASK-TDL-050"]);
    assert!(graph.items[0].target_files.is_empty());
    assert!(!graph.items[0].artifact_requirements.is_empty());
}

#[test]
fn review_remediation_accepts_split_gaps_for_same_canonical_task() {
    let fixture = include_str!("fixtures/wf28f_review_remediation_duplicate_task_items.json");
    let source_data: serde_json::Value = serde_json::from_str(fixture).expect("fixture json");
    let execution = review_remediation_execution(source_data);

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert_eq!(metadata.invalid_reason, None, "{metadata:?}");
    let graph = metadata.source_task_graph.expect("graph");
    assert_eq!(graph.items.len(), 2);
    assert!(graph.items.iter().all(|item| {
        item.canonical_task_ids == vec!["TASK-TDL-050"]
            && item.target_files.is_empty()
            && !item.artifact_requirements.is_empty()
    }));
}

#[test]
fn review_remediation_rejects_synthetic_task_ids() {
    let item = serde_json::json!({
        "item_id": "remediate-project-artifact",
        "canonical_task_ids": ["remediate-project-artifact"],
        "dependency_ids": [],
        "source_item_id": "review-gap-1",
        "failure_status": "missing_project_artifact",
        "failure_evidence": ["required project artifact is absent"],
        "required_fix": "create the missing project artifact under the runtime project root",
        "target_files": [],
        "focused_verification": ["verify the project artifact exists and matches schema"],
        "artifact_requirements": ["project .archon data artifact"]
    });
    let execution = review_remediation_execution(serde_json::json!([item]));

    let metadata = dynamic_wave_source_metadata(&execution, Some(&tdl_task_universe()), None);

    assert!(metadata.invalid_reason.is_some(), "{metadata:?}");
}

fn execution(source_data: serde_json::Value) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "implementation-wave-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".to_string()),
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({
            "objective": "Implement decomposed PRD",
            "source_data": source_data,
        }),
        depends_on: Vec::new(),
    }
}

fn review_remediation_execution(source_data: serde_json::Value) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "review-remediation-wave-1".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".to_string()),
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({
            "objective": "Run review remediation",
            "source_data": source_data,
        }),
        depends_on: Vec::new(),
    }
}

fn review_verification_execution(source_data: serde_json::Value) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "review-verification-wave-1".to_string(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options: WorkflowV2HostOptions {
                item_kind: Some("focused_verification".to_string()),
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({
            "objective": "Run review verification",
            "source_data": source_data,
        }),
        depends_on: Vec::new(),
    }
}

fn focused_verification_execution(source_data: serde_json::Value) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "verification-wave-1".to_string(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options: WorkflowV2HostOptions {
                item_kind: Some("focused_verification".to_string()),
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({
            "objective": "Run focused verification",
            "source_data": source_data,
        }),
        depends_on: Vec::new(),
    }
}

fn task_universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-001".to_string(),
                aliases: vec!["T001".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-001.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
            },
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-010".to_string(),
                aliases: vec!["T010".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-010.md".to_string(),
                dependency_ids: vec!["TASK-TDL-001".to_string()],
                title: None,
            },
        ],
    }
}

fn tdl_task_universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp/tasks".to_string()],
        tasks: vec![
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-001".to_string(),
                aliases: vec!["T001".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-001.md".to_string(),
                dependency_ids: Vec::new(),
                title: None,
            },
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-010".to_string(),
                aliases: vec!["T010".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-010.md".to_string(),
                dependency_ids: vec!["TASK-TDL-001".to_string()],
                title: None,
            },
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-020".to_string(),
                aliases: vec!["T020".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-020.md".to_string(),
                dependency_ids: vec!["TASK-TDL-010".to_string()],
                title: None,
            },
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-050".to_string(),
                aliases: vec!["T050".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-050.md".to_string(),
                dependency_ids: vec!["TASK-TDL-020".to_string()],
                title: None,
            },
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-TDL-070".to_string(),
                aliases: vec!["T070".to_string()],
                source_path: "/tmp/tasks/TASK-TDL-070.md".to_string(),
                dependency_ids: vec!["TASK-TDL-020".to_string()],
                title: None,
            },
        ],
    }
}
