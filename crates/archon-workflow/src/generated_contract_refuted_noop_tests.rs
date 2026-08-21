//! A no-op that verification already refuted cannot come back as a no-op.
//!
//! `noop_routing::implementation_item` stamps `noop_reclassification` onto
//! exactly the items whose no-op claim was tested and failed. The retry item
//! already carries the refutation — `required_fix`, `failure_evidence`, and the
//! refuted claim itself — into the agent's own prompt, and agents answered the
//! same way regardless. These pin the rule rather than the request.

use super::*;
use crate::task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};

/// No verifier command, so the execution rule cannot be what fires here. This
/// keeps the two rules independent: a failure below means the refutation rule
/// is doing the work, not the one it sits beside.
fn inspectable_task(id: &str) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        source_path: format!("/tmp/{id}.md"),
        deliverable_contracts: vec![WorkflowV2DeliverableContract {
            kind: "document".to_string(),
            artifact_path: "docs/report.md".to_string(),
            typed_verifier_command: None,
            registry_path: None,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn contract_for(id: &str) -> ContractTaskUniverse {
    ContractTaskUniverse::from_authoritative(Some(&WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp".to_string()],
        tasks: vec![inspectable_task(id)],
    }))
}

fn noop_item(task_id: &str, reclassification: Option<serde_json::Value>) -> serde_json::Value {
    let mut item = serde_json::json!({
        "item_id": "noop-1",
        "work_type": "verified_noop",
        "canonical_task_ids": [task_id],
        "acceptance_criteria": ["the report records every gap"],
        "noop_proof": "the document already exists",
        "noop_proof_refs": ["docs/report.md"],
        "artifact_requirements": ["docs/report.md"],
    });
    if let Some(value) = reclassification {
        item["noop_reclassification"] = value;
    }
    item
}

fn refutation() -> serde_json::Value {
    serde_json::json!({
        "count": 1,
        "source": "verification-wave-1",
        "refuted_claim": "the document already exists",
    })
}

fn issue_fields(issues: &[GeneratedContractIssue]) -> Vec<String> {
    issues.iter().map(|issue| issue.field.clone()).collect()
}

/// The live failure: refuted, returned, and re-proposed as the same no-op.
#[test]
fn a_refuted_noop_cannot_be_re_proposed_as_a_noop() {
    let issues = generated_item_issues(
        &noop_item("TASK-A-001", Some(refutation())),
        &contract_for("TASK-A-001"),
        None,
    );

    assert!(
        issue_fields(&issues).contains(&"work_type".to_string()),
        "expected the refutation to bar a second no-op, got: {:?}",
        issue_fields(&issues)
    );
}

/// A first-time no-op carries no refutation and must stay available: the rule
/// bars repeating a lost argument, not making one.
#[test]
fn a_first_time_noop_is_still_allowed() {
    let issues = generated_item_issues(
        &noop_item("TASK-A-001", None),
        &contract_for("TASK-A-001"),
        None,
    );

    assert!(
        !issue_fields(&issues).contains(&"work_type".to_string()),
        "a never-refuted no-op must not be barred, got: {:?}",
        issue_fields(&issues)
    );
}

/// An empty marker is not a refutation. `value_present` decides this, and the
/// test pins it so a later change to that helper cannot silently disarm the
/// rule.
#[test]
fn an_empty_reclassification_marker_does_not_bar_a_noop() {
    let issues = generated_item_issues(
        &noop_item("TASK-A-001", Some(serde_json::json!({}))),
        &contract_for("TASK-A-001"),
        None,
    );

    assert!(
        !issue_fields(&issues).contains(&"work_type".to_string()),
        "an empty marker must not bar a no-op, got: {:?}",
        issue_fields(&issues)
    );
}

/// The reclassified item's own route out: implementation work is unaffected.
#[test]
fn the_same_item_as_implementation_work_is_not_barred() {
    let mut item = noop_item("TASK-A-001", Some(refutation()));
    item["work_type"] = serde_json::json!("implementation");
    item["target_files"] = serde_json::json!(["docs/report.md"]);

    let issues = generated_item_issues(&item, &contract_for("TASK-A-001"), None);

    assert!(
        !issue_fields(&issues).contains(&"work_type".to_string()),
        "implementation is the route the rule points at, got: {:?}",
        issue_fields(&issues)
    );
}
