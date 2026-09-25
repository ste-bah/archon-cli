use super::*;

use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind};
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2WriteMode};

fn review_call(stage: &str) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "reviewContract".to_string(),
        serde_json::json!({ "version": 1, "kind": "uncovered_requirements", "stage": stage, "findingsPath": "data.findings" }),
    );
    WorkflowV2HostCall {
        id: "coverage-audit-map".to_string(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options,
    }
}

fn review_result(status: WorkflowV2Status, findings: Option<Value>) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("reviewed the task");
    result.status = status;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "read the task's files",
    ));
    if let Some(findings) = findings {
        result.data = serde_json::json!({ "findings": findings });
    }
    result
}

fn outcome(
    status: WorkflowV2Status,
    failure_kind: Option<BranchFailureKind>,
    result: Option<WorkflowV2Result>,
) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: "coverage-audit-map-12".to_string(),
        role: "critic".to_string(),
        status,
        result,
        error: None,
        failure_kind,
        item_input_hash: Some("identity".to_string()),
        completion_evidence: Vec::new(),
    }
}

fn finished_with_findings() -> WorkflowV2BranchOutcome {
    outcome(
        WorkflowV2Status::NeedsReview,
        Some(BranchFailureKind::Semantic),
        Some(review_result(
            WorkflowV2Status::NeedsReview,
            Some(serde_json::json!([{ "id": "gap-1", "claim": "REQ-7 uncovered" }])),
        )),
    )
}

#[test]
fn a_review_that_finished_with_findings_is_a_completed_branch() {
    assert!(completed_review_branch(
        &review_call("map"),
        &finished_with_findings()
    ));
}

#[test]
fn a_review_branch_the_host_got_no_answer_from_is_not_completed() {
    let call = review_call("map");
    let mut contract = finished_with_findings();
    contract.failure_kind = Some(BranchFailureKind::Contract);
    assert!(!completed_review_branch(&call, &contract));
    let mut safety = finished_with_findings();
    safety.failure_kind = Some(BranchFailureKind::Safety);
    assert!(!completed_review_branch(&call, &safety));
    let transport = outcome(
        WorkflowV2Status::Failed,
        Some(BranchFailureKind::Execution),
        None,
    );
    assert!(!completed_review_branch(&call, &transport));
    let mut errored = finished_with_findings();
    errored.error = Some("agent transport failed".to_string());
    assert!(!completed_review_branch(&call, &errored));
    let no_findings = outcome(
        WorkflowV2Status::NeedsReview,
        Some(BranchFailureKind::Semantic),
        Some(review_result(WorkflowV2Status::NeedsReview, None)),
    );
    assert!(
        !completed_review_branch(&call, &no_findings),
        "a needs_review verdict without the contract's findings array is no finished review"
    );
    let mut unhashed = finished_with_findings();
    unhashed.item_input_hash = None;
    assert!(!completed_review_branch(&call, &unhashed));
}

#[test]
fn only_a_read_only_review_map_qualifies() {
    let outcome = finished_with_findings();
    assert!(!completed_review_branch(
        &review_call("reduce_final"),
        &outcome
    ));
    let mut write = review_call("map");
    write.write_mode = Some(WorkflowV2WriteMode::Worktree);
    assert!(!completed_review_branch(&write, &outcome));
    let mut plain = review_call("map");
    plain.options.extra.clear();
    assert!(
        !completed_review_branch(&plain, &outcome),
        "a verifier's needs_review is a rejection, not a review's findings"
    );
}

fn map_record(views: Vec<Value>, attached: bool) -> WorkflowV2CallRecord {
    let mut result = review_result(WorkflowV2Status::NeedsReview, None);
    result.data = serde_json::json!({ "outcomes": views });
    if attached {
        result.data["review_findings"] = serde_json::json!({ "source": "host", "findings": [] });
    }
    WorkflowV2CallRecord::new(
        "run",
        review_call("map"),
        2,
        "input".to_string(),
        result,
        Vec::new(),
    )
}

fn view(outcome: &WorkflowV2BranchOutcome) -> Value {
    serde_json::to_value(outcome).expect("view")
}

fn accepted_view() -> Value {
    view(&outcome(
        WorkflowV2Status::Accepted,
        None,
        Some(review_result(
            WorkflowV2Status::Accepted,
            Some(serde_json::json!([])),
        )),
    ))
}

#[test]
fn a_map_whose_every_branch_finished_its_review_is_reusable() {
    let record = map_record(vec![accepted_view(), view(&finished_with_findings())], true);
    assert!(completed_review_map_record(&record));
    assert!(record.is_reusable_for("input"));
    assert!(!record.is_reusable_for("other input"));
    let mut invalidated = record.clone();
    invalidated.invalidated_by = Some("restart-stage".to_string());
    assert!(!invalidated.is_reusable_for("input"));
}

#[test]
fn a_map_with_a_failed_or_unattached_branch_set_is_not_reusable() {
    let failed = view(&outcome(
        WorkflowV2Status::Failed,
        Some(BranchFailureKind::Execution),
        None,
    ));
    let record = map_record(vec![accepted_view(), failed], true);
    assert!(!completed_review_map_record(&record));
    assert!(!record.is_reusable_for("input"));
    let unattached = map_record(vec![view(&finished_with_findings())], false);
    assert!(
        !completed_review_map_record(&unattached),
        "a map record without the host's finding attachment was never contract-checked"
    );
    assert!(!completed_review_map_record(&map_record(Vec::new(), true)));
    let mut not_a_map = map_record(vec![view(&finished_with_findings())], true);
    not_a_map.call = review_call("reduce_final");
    assert!(!not_a_map.is_reusable_for("input"));
}

#[test]
fn a_review_the_agent_called_partial_is_not_finished() {
    for raw in [
        "partial",
        "partial_success",
        "incomplete",
        "completed_with_gaps",
        "accepted_with_gaps",
    ] {
        let mut result = review_result(
            WorkflowV2Status::NeedsReview,
            Some(serde_json::json!([{ "id": "gap-1" }])),
        );
        stamp_partial_status(&serde_json::json!({ "status": raw }), &mut result);
        assert_eq!(
            result.data[AGENT_REPORTED_STATUS_KEY],
            serde_json::json!(raw)
        );
        let branch = outcome(
            WorkflowV2Status::NeedsReview,
            Some(BranchFailureKind::Semantic),
            Some(result),
        );
        assert!(
            !completed_review_branch(&review_call("map"), &branch),
            "{raw}"
        );
        let record = map_record(vec![view(&branch)], true);
        assert!(!completed_review_map_record(&record), "{raw}");
    }
    let mut plain = review_result(WorkflowV2Status::NeedsReview, Some(serde_json::json!([])));
    stamp_partial_status(&serde_json::json!({ "status": "needs_review" }), &mut plain);
    assert!(
        plain.data.get(AGENT_REPORTED_STATUS_KEY).is_none(),
        "a plain verdict is not stamped"
    );
}

#[test]
fn a_map_record_missing_a_dispatched_branch_is_not_reusable() {
    let mut record = map_record(vec![view(&finished_with_findings())], true);
    record.dispatched_items = vec![
        crate::v2::result_store::WorkflowV2DispatchedItem {
            item_id: "coverage-audit-map-12".to_string(),
            canonical_task_ids: vec!["TASK-12".to_string()],
        },
        crate::v2::result_store::WorkflowV2DispatchedItem {
            item_id: "coverage-audit-map-13".to_string(),
            canonical_task_ids: vec!["TASK-13".to_string()],
        },
    ];
    assert!(
        !completed_review_map_record(&record),
        "branch 13 was dispatched and never answered"
    );
    record.dispatched_items.pop();
    assert!(completed_review_map_record(&record));
}

#[test]
fn the_adapter_keeps_the_partial_word_an_agent_wrote() {
    let mut call = review_call("map");
    call.id = "coverage-audit-map-1".to_string();
    call.method = WorkflowV2HostMethod::Agent;
    let execution = crate::v2::call_execution::WorkflowV2CallExecution {
        call,
        input: serde_json::json!({ "item": { "item_id": "review-task-1", "canonical_task_ids": ["TASK-1"] } }),
        depends_on: Vec::new(),
    };
    let request = crate::v2::call_data::v2_agent_request("review", None, &execution, None);
    let output = serde_json::json!({
        "status": "partial", "summary": "reviewed half the task",
        "evidence": [{ "kind": "review", "summary": "read half of the files" }],
        "data": { "findings": [] },
    });
    let parsed = crate::v2::agent_adapter::WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output.to_string())
        .expect("a partial review parses");
    assert_eq!(parsed.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        parsed.data[AGENT_REPORTED_STATUS_KEY],
        serde_json::json!("partial")
    );
}
