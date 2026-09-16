//! Obs-22: the reduce input names every branch its source maps ran, and the
//! prompt says how to read a zero. Replays the live shape: a map whose
//! branches were all accepted, some with findings and some with none.

use std::collections::BTreeMap;

use super::{BRANCH_ROSTER_KEY, BRANCH_ROSTER_RULE, attach_branch_roster, roster_entry};
use crate::v2::call_data::{execution_with_resolved_source, v2_agent_request};
use crate::v2::{
    WorkflowV2AgentAdapter, WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2CallRecord,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status,
};

fn store() -> (tempfile::TempDir, WorkflowV2ResultStore) {
    let dir = tempfile::tempdir().expect("tmp");
    let store = WorkflowV2ResultStore::new(dir.path().join("v2"));
    (dir, store)
}

fn branch(item_id: &str, task: &str, findings: usize) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result::accepted("reviewed");
    let findings: Vec<_> = (0..findings)
        .map(|n| serde_json::json!({ "id": format!("{item_id}-F{n}"), "severity": "low" }))
        .collect();
    // The post-Obs-22 stamp: ids on `data`, beside the agent's findings.
    result.data = serde_json::json!({ "findings": findings, "canonical_task_ids": [task] });
    WorkflowV2BranchOutcome {
        item_id: item_id.to_string(),
        role: "critic".to_string(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

fn reduce_call(sources: serde_json::Value) -> WorkflowV2HostCall {
    let extra: BTreeMap<String, serde_json::Value> = [(
        "reviewContract".to_string(),
        serde_json::json!({
            "kind": "adversarial_findings", "stage": "reduce_final",
            "sourceMapCallIds": sources, "preserveMapFindings": true, "maxInputBytes": 48000,
        }),
    )]
    .into();
    WorkflowV2HostCall {
        id: "adversarial-review-reduce".to_string(),
        method: WorkflowV2HostMethod::Reduce,
        write_mode: None,
        options: WorkflowV2HostOptions {
            role: Some("critic".to_string()),
            extra,
            ..Default::default()
        },
    }
}

fn reduce_execution(sources: serde_json::Value) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: reduce_call(sources),
        // What `reviewMapReduce` sends: the host-attributed map findings only.
        input: serde_json::json!({ "findings": [{ "id": "map-1-F0" }, { "id": "map-1-F1" }] }),
        depends_on: Vec::new(),
    }
}

#[test]
fn the_reduce_input_carries_every_source_branch_with_its_finding_count() {
    let (_dir, store) = store();
    store
        .save_branch_outcome("adversarial-review-map", &branch("map-1", "TASK-001", 2))
        .expect("save");
    store
        .save_branch_outcome("adversarial-review-map", &branch("map-2", "TASK-002", 0))
        .expect("save");
    let execution = reduce_execution(serde_json::json!(["adversarial-review-map"]));

    let dispatched = execution_with_resolved_source(&execution, &store).expect("resolve");

    assert_eq!(
        dispatched.input[BRANCH_ROSTER_KEY],
        serde_json::json!([
            { "item_id": "map-1", "canonical_task_ids": ["TASK-001"], "status": "accepted", "finding_count": 2 },
            { "item_id": "map-2", "canonical_task_ids": ["TASK-002"], "status": "accepted", "finding_count": 0 },
        ])
    );
    // The findings the script sent are untouched beside it.
    assert_eq!(
        dispatched.input["findings"].as_array().map(Vec::len),
        Some(2)
    );

    let request = v2_agent_request("review", None, &dispatched, None);
    assert!(
        request.constraints.iter().any(|c| c == BRANCH_ROSTER_RULE),
        "the roster rule must be a typed constraint: {:?}",
        request.constraints
    );
    let prompt = WorkflowV2AgentAdapter::new().build_prompt(&request);
    assert!(
        prompt.contains("branch_roster"),
        "roster must reach the prompt input"
    );
    assert!(
        prompt.contains("a zero finding_count means reviewed with nothing to report"),
        "the prompt must say what a zero means"
    );
}

#[test]
fn a_call_without_a_review_contract_gets_no_roster_and_no_rule() {
    let (_dir, store) = store();
    store
        .save_branch_outcome("some-map", &branch("map-1", "TASK-001", 1))
        .expect("save");
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "plain-reduce".to_string(),
            method: WorkflowV2HostMethod::Reduce,
            write_mode: None,
            options: Default::default(),
        },
        input: serde_json::json!({ "findings": [] }),
        depends_on: Vec::new(),
    };
    let dispatched = execution_with_resolved_source(&execution, &store).expect("resolve");
    assert!(dispatched.input.get(BRANCH_ROSTER_KEY).is_none());
    let request = v2_agent_request("review", None, &dispatched, None);
    assert!(!request.constraints.iter().any(|c| c == BRANCH_ROSTER_RULE));
}

#[test]
fn the_host_roster_replaces_the_scripts_best_effort_one() {
    let (_dir, store) = store();
    store
        .save_branch_outcome("adversarial-review-map", &branch("map-1", "TASK-001", 3))
        .expect("save");
    let mut execution = reduce_execution(serde_json::json!(["adversarial-review-map"]));
    execution.input[BRANCH_ROSTER_KEY] = serde_json::json!([{ "item_id": "map-1", "canonical_task_ids": [], "status": "accepted", "finding_count": 0 }]);
    attach_branch_roster(&mut execution, &store).expect("attach");
    assert_eq!(
        execution.input[BRANCH_ROSTER_KEY][0]["finding_count"],
        serde_json::json!(3)
    );
    assert_eq!(
        execution.input[BRANCH_ROSTER_KEY][0]["canonical_task_ids"],
        serde_json::json!(["TASK-001"])
    );
}

#[test]
fn a_source_with_no_records_leaves_the_scripts_roster_alone() {
    let (_dir, store) = store();
    let mut execution = reduce_execution(serde_json::json!(["never-ran-map"]));
    let script_roster = serde_json::json!([{ "item_id": "x", "canonical_task_ids": ["T"], "status": "accepted", "finding_count": 0 }]);
    execution.input[BRANCH_ROSTER_KEY] = script_roster.clone();
    attach_branch_roster(&mut execution, &store).expect("attach");
    assert_eq!(execution.input[BRANCH_ROSTER_KEY], script_roster);
}

/// A map whose branch files are absent but whose call record carries the
/// outcome views (the same outcomes serialised) still yields a roster.
#[test]
fn the_call_record_outcomes_back_the_branch_files() {
    let (_dir, store) = store();
    let mut map_result = WorkflowV2Result::accepted("fanout done");
    map_result.data = serde_json::json!({
        "outcomes": [
            { "item_id": "map-1", "id": "map-1", "role": "critic", "status": "accepted",
              "canonical_task_ids": ["TASK-001"], "evidence": [],
              "result": branch("map-1", "TASK-001", 1).result, "error": null, "completion_evidence": [] },
            { "item_id": "map-2", "id": "map-2", "role": "critic", "status": "failed",
              "canonical_task_ids": [], "evidence": [], "result": null,
              "error": "timed out", "completion_evidence": [] },
        ],
        "items": [],
    });
    let map_call = WorkflowV2HostCall {
        id: "adversarial-review-map".to_string(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options: Default::default(),
    };
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            "run",
            map_call,
            1,
            "hash".to_string(),
            map_result,
            Vec::new(),
        ))
        .expect("save record");
    let mut execution = reduce_execution(serde_json::json!(["adversarial-review-map"]));
    attach_branch_roster(&mut execution, &store).expect("attach");
    assert_eq!(
        execution.input[BRANCH_ROSTER_KEY],
        serde_json::json!([
            { "item_id": "map-1", "canonical_task_ids": ["TASK-001"], "status": "accepted", "finding_count": 1 },
            { "item_id": "map-2", "canonical_task_ids": [], "status": "failed", "finding_count": 0 },
        ])
    );
}

/// A record saved before the read-only stamp existed: ids only in accepted
/// task coverage, which the wider result view still reads.
#[test]
fn a_pre_stamp_outcome_falls_back_to_task_coverage() {
    let mut outcome = branch("map-4", "TASK-005", 0);
    let result = outcome.result.as_mut().unwrap();
    result.data = serde_json::json!({ "findings": [] });
    result
        .task_coverage
        .push(crate::v2::WorkflowV2TaskCoverage {
            task_id: "TASK-005".to_string(),
            status: crate::v2::WorkflowV2TaskCoverageStatus::Accepted,
            summary: "reviewed".to_string(),
            evidence: Vec::new(),
        });
    let entry = roster_entry(&outcome);
    assert_eq!(entry.canonical_task_ids, vec!["TASK-005".to_string()]);
    assert_eq!(entry.finding_count, 0);
}

#[test]
fn a_non_object_reduce_input_is_wrapped_beside_the_roster() {
    let (_dir, store) = store();
    store
        .save_branch_outcome("adversarial-review-map", &branch("map-1", "TASK-001", 0))
        .expect("save");
    let mut execution = reduce_execution(serde_json::json!(["adversarial-review-map"]));
    execution.input = serde_json::json!(["bare", "array"]);
    attach_branch_roster(&mut execution, &store).expect("attach");
    assert_eq!(
        execution.input["input"],
        serde_json::json!(["bare", "array"])
    );
    assert_eq!(
        execution.input[BRANCH_ROSTER_KEY][0]["item_id"],
        serde_json::json!("map-1")
    );
}
