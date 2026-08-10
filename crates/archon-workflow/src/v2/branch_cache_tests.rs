use super::*;

use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result};
use crate::v2::scheduler::BranchFailureKind;
use crate::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};

fn accepted_result() -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("branch produced the declared change");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "branch recorded concrete implementation evidence",
    ));
    result
}

fn item(call_id: &str, item_id: &str) -> WorkflowV2FanoutItem {
    let call = WorkflowV2HostCall {
        id: format!("{call_id}-{item_id}"),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    WorkflowV2FanoutItem::read_only(
        format!("{call_id}-{item_id}"),
        "coder",
        call,
        serde_json::json!({
            "fanout_call_id": call_id,
            "fanout_item_id": item_id,
            "item": {"id": item_id, "target_files": ["src/lib.rs"]},
        }),
    )
}

fn accepted_outcome(item: &WorkflowV2FanoutItem) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: item.id.clone(),
        role: item.role.clone(),
        status: WorkflowV2Status::Accepted,
        result: Some(accepted_result()),
        error: None,
        failure_kind: None,
        item_input_hash: Some(item.input_hash()),
        completion_evidence: Vec::new(),
    }
}

#[test]
fn accepted_outcome_is_reused_for_the_same_call_and_item() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let item = item("remediation-wave-1", "task-a");
    store
        .save_branch_outcome("remediation-wave-1", &accepted_outcome(&item))
        .expect("save outcome");

    // Not a wave call id for the completion-evidence rule, so the accepted
    // outcome is reusable on its own terms.
    let (reused, pending) =
        split_reusable_branch_outcomes(&store, "restartable-fanout", vec![item.clone()])
            .expect("split");

    assert!(reused.is_empty(), "different call id must not match");
    assert_eq!(pending.len(), 1);

    store
        .save_branch_outcome("restartable-fanout", &accepted_outcome(&item))
        .expect("save outcome");
    let (reused, pending) =
        split_reusable_branch_outcomes(&store, "restartable-fanout", vec![item]).expect("split");

    assert_eq!(reused.len(), 1);
    assert!(pending.is_empty());
}

/// The retry wave carries only work that did NOT resolve, so an outcome stored
/// under the previous attempt's call id must stay invisible to it — even when
/// the retry re-derives a byte-identical item payload. See this module's header
/// for why widening the key would let a review loop credit a fix that did not
/// stick.
#[test]
fn cross_attempt_reuse_is_refused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let first = item("remediation-wave-1", "task-a");
    let mut outcome = accepted_outcome(&first);
    outcome.completion_evidence = Vec::new();
    store
        .save_branch_outcome("remediation-wave-1", &outcome)
        .expect("save outcome");

    // Same item payload, retried under the call id the lifecycle driver mints
    // for the follow-up wave.
    let retried = item("remediation-wave-1-1", "task-a");

    let (reused, pending) =
        split_reusable_branch_outcomes(&store, "remediation-wave-1-1", vec![retried])
            .expect("split");

    assert!(reused.is_empty());
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "remediation-wave-1-1-task-a");
}

/// A stored outcome that claims `accepted` while carrying a failure kind is
/// self-contradicting. Reuse must take the pessimistic reading: crediting failed
/// work as done is unrecoverable, re-running it is merely expensive.
#[test]
fn accepted_outcome_carrying_a_failure_kind_is_not_reused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let item = item("restartable-fanout", "task-a");
    let mut outcome = accepted_outcome(&item);
    outcome.failure_kind = Some(BranchFailureKind::Safety);
    store
        .save_branch_outcome("restartable-fanout", &outcome)
        .expect("save outcome");

    let (reused, pending) =
        split_reusable_branch_outcomes(&store, "restartable-fanout", vec![item]).expect("split");

    assert!(reused.is_empty());
    assert_eq!(pending.len(), 1);
}

#[test]
fn wave_call_ids_require_completion_evidence_before_reuse() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let item = item("implementation-wave-1", "task-a");
    store
        .save_branch_outcome("implementation-wave-1", &accepted_outcome(&item))
        .expect("save outcome");

    let (reused, pending) =
        split_reusable_branch_outcomes(&store, "implementation-wave-1", vec![item]).expect("split");

    assert!(reused.is_empty());
    assert_eq!(pending.len(), 1);
}
