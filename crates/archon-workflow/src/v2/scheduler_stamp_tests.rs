//! Obs-22 (run wf-719ff3b0): every read-only branch outcome names the task
//! its item owns, whether or not the agent echoed it.
//!
//! Two review branches (`adversarial-review-map-4`, `-8`) returned zero
//! findings and no `canonical_task_ids`; the saved outcomes could not say
//! which task each had reviewed, and the reducer concluded neither task had
//! been reviewed at all. The write path has stamped this since TD-058; these
//! pin that the read-only funnel now does the same, and only fills a gap —
//! an id set the agent returned is its own.

use super::{WorkflowV2FanoutItem, WorkflowV2Scheduler, WorkflowV2SchedulerConfig};
use crate::v2::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2Result, WorkflowV2Status,
};

fn call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options: Default::default(),
    }
}

/// The exact input shape the live map built: `canonical_task_ids` on the item
/// the host nests under `item`.
fn item(id: &str, task: &str) -> WorkflowV2FanoutItem {
    WorkflowV2FanoutItem::read_only(
        id,
        "critic",
        call(id),
        serde_json::json!({ "item": { "item_id": id, "canonical_task_ids": [task] } }),
    )
}

/// An accepted, valid result whose `data` carries what the agent chose.
fn accepted(data: serde_json::Value) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("reviewed");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "read the task",
    ));
    result.data = data;
    result
}

fn ids(outcome: &super::WorkflowV2BranchOutcome) -> serde_json::Value {
    outcome
        .result
        .as_ref()
        .and_then(|result| result.data.get("canonical_task_ids").cloned())
        .unwrap_or(serde_json::Value::Null)
}

#[tokio::test]
async fn a_read_only_branch_with_no_ids_is_stamped_from_its_item_input() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig::default());
    // Zero findings, no ids: the live map-4 / map-8 shape.
    let report = scheduler
        .run_read_only_fanout(vec![item("map-4", "TASK-005")], |_| async {
            Ok(accepted(serde_json::json!({ "findings": [] })))
        })
        .await
        .expect("fanout runs");
    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(report.outcomes[0].status, WorkflowV2Status::Accepted);
    assert_eq!(ids(&report.outcomes[0]), serde_json::json!(["TASK-005"]));
    // The agent's own data survives beside the stamp.
    assert_eq!(
        report.outcomes[0].result.as_ref().unwrap().data["findings"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn a_read_only_branch_that_returned_ids_keeps_them() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig::default());
    let report = scheduler
        .run_read_only_fanout(vec![item("map-1", "TASK-001")], |_| async {
            // A reviewer that legitimately names two tasks keeps both.
            Ok(accepted(
                serde_json::json!({ "canonical_task_ids": ["TASK-001", "TASK-002"] }),
            ))
        })
        .await
        .expect("fanout runs");
    assert_eq!(
        ids(&report.outcomes[0]),
        serde_json::json!(["TASK-001", "TASK-002"])
    );
}

#[tokio::test]
async fn a_non_object_data_is_replaced_by_the_stamp_and_an_empty_array_is_filled() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig::default());
    let report = scheduler
        .run_read_only_fanout(
            vec![item("map-2", "TASK-002"), item("map-3", "TASK-003")],
            |branch| async move {
                Ok(if branch.id == "map-2" {
                    accepted(serde_json::Value::Null)
                } else {
                    accepted(serde_json::json!({ "canonical_task_ids": [] }))
                })
            },
        )
        .await
        .expect("fanout runs");
    let by_id = |id: &str| {
        report
            .outcomes
            .iter()
            .find(|outcome| outcome.item_id == id)
            .expect("outcome present")
    };
    assert_eq!(ids(by_id("map-2")), serde_json::json!(["TASK-002"]));
    assert_eq!(ids(by_id("map-3")), serde_json::json!(["TASK-003"]));
}

#[tokio::test]
async fn a_failed_branch_is_not_given_a_result_to_stamp() {
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig::default());
    let report = scheduler
        .run_read_only_fanout(vec![item("map-9", "TASK-009")], |_| async {
            Err(crate::WorkflowError::StageFailed(
                "provider down".to_string(),
            ))
        })
        .await
        .expect("fanout runs");
    assert_eq!(report.outcomes[0].status, WorkflowV2Status::Failed);
    assert!(report.outcomes[0].result.is_none());
}
