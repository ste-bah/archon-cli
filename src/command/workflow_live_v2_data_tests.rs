use std::collections::BTreeMap;

use archon_workflow::{
    RetryPolicy, StageKind, WorkflowSpec, WorkflowV2BranchOutcome, WorkflowV2CallExecution,
    WorkflowV2CallRecord, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutReport,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status, WorkflowV2WriteMode,
};

use super::workflow_live_v2_data::{
    fanout_items_for_call, result_from_fanout_report, v2_agent_request,
};

#[test]
fn empty_fanout_without_noop_proof_becomes_review_input() {
    let result = result_from_fanout_report(&fanout_call("review-required-tasks"), report(vec![]));

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(result.summary.contains("zero items"));
    assert_eq!(
        result.residual_gaps[0].id,
        "empty_fanout_review-required-tasks"
    );
}

#[test]
fn read_only_fanout_blocked_branch_blocks_downstream_work() {
    let result = result_from_fanout_report(
        &fanout_call("readOnlyAudits"),
        report(vec![accepted_outcome("a"), blocked_outcome("b")]),
    );

    assert_eq!(result.status, WorkflowV2Status::Blocked);
    assert!(result.summary.contains("blocked"));
    let items = result.data["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1]["status"], "blocked");
    assert_eq!(items[1]["residual_gaps"][0]["severity"], "blocking");
}

#[test]
fn read_only_fanout_review_branch_stays_needs_review_not_failed() {
    let result = result_from_fanout_report(
        &fanout_call("readOnlyAudits"),
        report(vec![accepted_outcome("a"), review_outcome("b")]),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(result.summary.contains("needing review"));
    assert_eq!(result.data["items"].as_array().unwrap().len(), 2);
}

#[test]
fn schema_branch_error_fails_fanout_before_downstream_work() {
    let result = result_from_fanout_report(
        &fanout_call("readOnlyAudits"),
        report(vec![
            accepted_outcome("a"),
            failed_error_outcome(
                "b",
                "schema repair failed after one retry: first=agent output contains a confirmation question instead of executing; repair=agent result failed validation: workflow result summary is required",
            ),
        ]),
    );

    assert_eq!(result.status, WorkflowV2Status::Failed);
    let items = result.data["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1]["status"], "failed");
    assert_eq!(items[1]["data"]["terminal_from_error"], true);
    assert_eq!(items[1]["residual_gaps"][0]["severity"], "blocking");
}

#[test]
fn transport_branch_error_fails_fanout_before_reducer_or_human_gate() {
    let result = result_from_fanout_report(
        &fanout_call("readOnlyAudits"),
        report(vec![
            accepted_outcome("a"),
            failed_error_outcome("b", "agent transport failed: rate limit"),
        ]),
    );

    assert_eq!(result.status, WorkflowV2Status::Failed);
    assert!(result.summary.contains("failed"));
    assert_eq!(result.data["items"][1]["status"], "failed");
}

#[test]
fn item_producer_request_demands_flat_items_array() {
    let mut extra = BTreeMap::new();
    extra.insert("outputs".to_string(), serde_json::json!(["items"]));
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "test".to_string(),
        task: "Implement a decomposed PRD.".to_string(),
        target_repository_root: Some("/repo".to_string()),
        max_parallelism: 8,
        max_agents: 32,
        provider_tiers: BTreeMap::new(),
        stages: vec![archon_workflow::StageSpec {
            id: "discover".to_string(),
            kind: StageKind::Agent,
            task: Some("Discover work items.".to_string()),
            agent: None,
            foreach: None,
            reducer: None,
            tool: None,
            condition: None,
            depends_on: Vec::new(),
            provider_tier: None,
            retry: RetryPolicy::default(),
            input: serde_json::Value::Null,
            model: None,
            provider: None,
            expected_target_files: Vec::new(),
            verify_command: None,
            max_parallelism: None,
            item_kind: None,
            filter: None,
            extra,
        }],
        artifact_policy: Default::default(),
        permissions: BTreeMap::new(),
        quality_gates: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "discover".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        input: serde_json::Value::Null,
        depends_on: Vec::new(),
    };

    let request = v2_agent_request("objective", &spec, &execution);

    assert!(
        request
            .constraints
            .iter()
            .any(|constraint| constraint.contains("data.items as a flat JSON array"))
    );
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
    let request = v2_agent_request("objective", &spec, &branch_execution);

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
    let request = v2_agent_request("objective", &spec, &branch_execution);

    assert_eq!(
        request.target_files,
        vec!["crates/archon-trading/src/data_lake.rs"]
    );
}

fn fanout_call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: None,
        options: Default::default(),
    }
}

fn report(outcomes: Vec<WorkflowV2BranchOutcome>) -> WorkflowV2FanoutReport {
    WorkflowV2FanoutReport {
        outcomes,
        max_parallelism: 8,
        peak_parallelism: 2,
        cancelled: false,
    }
}

fn accepted_outcome(id: &str) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result::accepted(format!("{id} accepted"));
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "branch inspected concrete input",
    ));
    outcome(id, WorkflowV2Status::Accepted, Some(result), None)
}

fn review_outcome(id: &str) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!("{id} needs review"),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "branch found a concrete review item",
    ));
    outcome(id, WorkflowV2Status::NeedsReview, Some(result), None)
}

fn blocked_outcome(id: &str) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Blocked,
        summary: format!("{id} blocked"),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Blocker,
        "branch found a concrete blocker",
    ));
    result
        .residual_gaps
        .push(archon_workflow::WorkflowV2ResidualGap {
            id: format!("{id}-gap"),
            description: "missing concrete artifact".to_string(),
            severity: Some("blocking".to_string()),
        });
    outcome(id, WorkflowV2Status::Blocked, Some(result), None)
}

fn failed_error_outcome(id: &str, error: &str) -> WorkflowV2BranchOutcome {
    outcome(id, WorkflowV2Status::Failed, None, Some(error.to_string()))
}

fn outcome(
    id: &str,
    status: WorkflowV2Status,
    result: Option<WorkflowV2Result>,
    error: Option<String>,
) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: id.to_string(),
        role: "researcher".to_string(),
        status,
        result,
        error,
    }
}
