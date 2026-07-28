use std::collections::BTreeMap;

use archon_workflow::{
    WorkflowSpec, WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2CallRecord,
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutReport, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus, WorkflowV2WriteMode,
    WorkflowV2AgentAdapter,
};

use super::super::workflow_live_task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};
use super::workflow_live_v2_data::{
    fanout_items_for_call, result_from_fanout_report, source_pack_value, v2_agent_request,
};

#[test]
fn fanout_builder_strips_forged_tool_declarations_from_read_only_branches() {
    // A read-only (verifier) fanout item forges tool declarations, including a
    // nested one. The shared builder must strip them so no MCP tool can bind
    // on a read-only branch by forgery.
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "verification-wave-1".to_string(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        input: serde_json::json!({
            "source_data": [{
                "item_id": "verify-1",
                "required_tools": ["forged_top"],
                "evidence": { "mcp_tools": ["forged_nested"] }
            }]
        }),
        depends_on: Vec::new(),
    };

    let branches = fanout_items_for_call(&execution, &store).expect("fanout items");
    let item = &branches[0].input["item"];

    assert!(item.get("required_tools").is_none(), "{item}");
    assert!(
        item["evidence"].get("mcp_tools").is_none(),
        "nested forgery must be stripped: {item}"
    );
}

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
fn read_only_fanout_structured_blocked_branch_continues_as_findings() {
    let result = result_from_fanout_report(
        &fanout_call("readOnlyAudits"),
        report(vec![accepted_outcome("a"), blocked_outcome("b")]),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(
        result
            .summary
            .contains("workflow.js to reduce or remediate")
    );
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
    assert!(
        result
            .summary
            .contains("workflow.js to reduce or remediate")
    );
    assert_eq!(result.data["items"].as_array().unwrap().len(), 2);
}

#[test]
fn read_only_fanout_unstructured_blocked_branch_with_sibling_continues_as_findings() {
    let result = result_from_fanout_report(
        &fanout_call("readOnlyAudits"),
        report(vec![
            accepted_outcome("a"),
            outcome("b", WorkflowV2Status::Blocked, None, None),
        ]),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(
        result
            .summary
            .contains("workflow.js to reduce or remediate")
    );
    assert_eq!(result.data["items"][1]["status"], "blocked");
}

#[test]
fn read_only_fanout_all_unstructured_blocked_branches_returns_review_data() {
    let result = result_from_fanout_report(
        &fanout_call("readOnlyAudits"),
        report(vec![outcome("b", WorkflowV2Status::Blocked, None, None)]),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(
        result
            .summary
            .contains("workflow.js to reduce or remediate")
    );
    assert_eq!(result.data["items"][0]["status"], "blocked");
}

#[test]
fn schema_branch_error_with_sibling_continues_as_findings() {
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

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(
        result
            .summary
            .contains("workflow.js to reduce or remediate")
    );
    let items = result.data["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1]["status"], "failed");
    assert_eq!(items[1]["data"]["branch_error_from_runtime"], true);
    assert_eq!(items[1]["residual_gaps"][0]["severity"], "blocking");
}

#[test]
fn transport_branch_error_with_sibling_continues_as_findings() {
    let result = result_from_fanout_report(
        &fanout_call("readOnlyAudits"),
        report(vec![
            accepted_outcome("a"),
            failed_error_outcome("b", "agent transport failed: rate limit"),
        ]),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(
        result
            .summary
            .contains("workflow.js to reduce or remediate")
    );
    assert_eq!(result.data["items"][1]["status"], "failed");
}

#[test]
fn read_only_fanout_all_failed_branches_returns_review_data() {
    let result = result_from_fanout_report(
        &fanout_call("readOnlyAudits"),
        report(vec![failed_error_outcome(
            "b",
            "agent transport failed: rate limit",
        )]),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(
        result
            .summary
            .contains("workflow.js to reduce or remediate")
    );
    assert_eq!(result.data["items"][0]["status"], "failed");
}

#[test]
fn source_pack_preserves_wrapper_and_outcome_contract_fields() {
    let packed = source_pack_value(&serde_json::json!({
        "kind": "implementation",
        "waveIndex": 1,
        "readyImplementationItems": [{
            "item_id": "impl-010",
            "canonical_task_ids": ["TASK-TDL-010"]
        }],
        "result": {
            "status": "needs_review",
            "summary": "one branch needs repair",
            "outcomes": [{
                "item_id": "impl-010",
                "canonical_task_ids": ["TASK-TDL-010"],
                "status": "needs_review",
                "failure_kind": "contract",
                "evidence": ["missing command evidence"],
                "artifact_paths": ["artifacts/report.json"],
                "commands_run": [{
                    "command": "cargo test focused",
                    "status": "failed",
                    "output_summary": "failed"
                }],
                "residual_gaps": [{
                    "id": "gap",
                    "severity": "blocking",
                    "description": "needs repair"
                }]
            }]
        }
    }));

    assert_eq!(packed["kind"], "implementation");
    assert_eq!(packed["waveIndex"], 1);
    assert_eq!(
        packed["readyImplementationItems"][0]["canonical_task_ids"][0],
        "TASK-TDL-010"
    );
    assert_eq!(packed["result"]["outcomes"][0]["item_id"], "impl-010");
    assert_eq!(
        packed["result"]["outcomes"][0]["canonical_task_ids"][0],
        "TASK-TDL-010"
    );
    assert_eq!(packed["result"]["outcomes"][0]["failure_kind"], "contract");
    assert_eq!(packed["result"]["outcome_count"], 1);
}

#[test]
fn implementation_fanout_downgrades_missing_branch_contract_to_needs_review() {
    let result = result_from_fanout_report(
        &implementation_fanout_call("implementation-wave-1"),
        report(vec![accepted_outcome("TASK-TDL-010")]),
    );

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data["outcomes"][0]["status"], "needs_review");
    assert_eq!(
        result.data["outcomes"][0]["canonical_task_ids"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(
        result.data["items"][0]["residual_gaps"][0]["id"]
            .as_str()
            .unwrap()
            .contains("invalid_implementation_branch_contract")
    );
}

#[test]
fn implementation_fanout_exposes_branch_contract_to_js() {
    let mut result = WorkflowV2Result::accepted("implemented registry schema");
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "TASK-TDL-010".to_string(),
        status: WorkflowV2TaskCoverageStatus::Accepted,
        summary: "registry schema accepted".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "changed crates/archon-trading/src/data_store.rs",
        )],
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Test,
        "cargo test -p archon-trading data_store_registry_schema",
    ));

    let result = result_from_fanout_report(
        &implementation_fanout_call("implementation-wave-1"),
        report(vec![outcome(
            "TASK-TDL-010",
            WorkflowV2Status::Accepted,
            Some(result),
            None,
        )]),
    );

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(result.data["outcomes"][0]["status"], "accepted");
    assert_eq!(
        result.data["outcomes"][0]["canonical_task_ids"][0],
        "TASK-TDL-010"
    );
    assert!(
        result.data["outcomes"][0]["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value
                .as_str()
                .is_some_and(|text| text.contains("data_store.rs")))
    );
}

#[test]
fn implementation_fanout_normalizes_task_and_evidence_aliases_to_js_outcome() {
    let mut branch = WorkflowV2Result::accepted("verified no-op implementation evidence");
    branch.data = serde_json::json!({
        "canonical_task_id": "TASK-TDL-060",
        "proof_references": ["src/trading/provider.rs:42"]
    });

    let result = result_from_fanout_report(
        &implementation_fanout_call("implementation-wave-1"),
        report(vec![outcome(
            "provider-native-candles",
            WorkflowV2Status::Accepted,
            Some(branch),
            None,
        )]),
    );

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(
        result.data["outcomes"][0]["canonical_task_ids"][0],
        "TASK-TDL-060"
    );
    assert!(
        result.data["outcomes"][0]["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value
                .as_str()
                .is_some_and(|text| text.contains("provider.rs:42")))
    );
}
