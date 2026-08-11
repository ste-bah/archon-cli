// #163 failure 2: the verification shape-repair loop could not converge.
//
// Three attempts, all rejected for one cause, then
// `blocked-verification-failed-1`. The guard was right every time; the loop was
// wrong to ask again, because appending the violations as issues is what keeps
// the loop condition true, so attempt N+1 sees the same items and the same
// reducer and can only return the same thing.
//
// Both properties under test are about how many times the loop ran and what it
// did instead, so a host that counts calls is what witnesses them.

use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::lifecycle_driver::review_test_host::RecordingHost;
use crate::v2::lifecycle_driver::{LifecycleEvidence, LifecycleLimits};
use crate::v2::lifecycle_policy::verify_routing::predicate_rewrite_inventory;
use crate::v2::semantic_preservation::check_items;

const ALLOWED_TASK: &str = "TASK-TDL-010";

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["tasks".to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: ALLOWED_TASK.to_string(),
            source_path: "tasks/TASK-TDL-010.md".to_string(),
            ..WorkflowV2TaskUniverseTask::default()
        }],
    }
}

fn driver(host: Arc<RecordingHost>, budget: u8) -> LifecycleDriver {
    LifecycleDriver::new(
        host,
        universe(),
        None,
        None,
        Value::Null,
        std::collections::BTreeSet::new(),
        LifecycleLimits {
            max_repair_iterations: budget,
            max_investigation_iterations: 1,
            implementation_wave_max_parallelism: Some(1),
        },
    )
}

/// The failed verification the retry inventory was built from. Its residual gap
/// is the identity the guard refuses to let a repair rewrite.
fn verification() -> Value {
    json!({
        "status": "failed",
        "outcomes": [{
            "item_id": "VERIFY-TDL-010-001",
            "status": "failed",
            "canonical_task_ids": [ALLOWED_TASK],
            "summary": "registry contract inspection failed",
            "residual_gaps": [{
                "id": "registry-schema-v2",
                "description": "registry schema_version is not archon-trading-data-registry-v2",
                "severity": "blocking",
            }],
        }],
    })
}

fn retry_item() -> Value {
    json!({
        "item_id": "VERIFY-TDL-010-001",
        "source_item_id": "VERIFY-TDL-010-001",
        "canonical_task_ids": [ALLOWED_TASK],
        "classification": "retryable_verification_shape_issue",
        "source_residual_gap_ids": ["registry-schema-v2"],
        "failed_predicate": "registry schema_version is not archon-trading-data-registry-v2",
        "focused_verification": ["re-run the registry artifact contract inspection"],
        "expected_evidence": ["schema=archon-trading-data-registry-v2"],
    })
}

/// A retry inventory in exactly the state `verify.rs` hands to the loop: it has
/// an item, and it has an outstanding issue, so the loop is entered.
fn repair_inventory() -> Value {
    json!({
        "status": "needs_review",
        "items": [retry_item()],
        "unresolved_issues": [{
            "kind": "evidence_repair",
            "field": "expected_evidence",
            "message": "retry item does not enumerate resolved artifact paths",
            "canonical_task_ids": [ALLOWED_TASK],
        }],
    })
}

/// What the reducer returned on every attempt of the observed run: the shape
/// issue addressed, and the item's classification rewritten along with it.
/// Reclassifying is the load-bearing identity change the guard exists to
/// refuse, so this is rejected however many times it is asked for.
fn rejected_shape_repair() -> Value {
    let mut item = retry_item();
    item["classification"] = json!("environment_dependency_missing");
    item["expected_evidence"] = json!([
        "schema=archon-trading-data-registry-v2",
        "migration_schema=archon-trading-data-registry-v2",
    ]);
    json!({ "status": "accepted", "items": [item], "unresolved_issues": [] })
}

/// The reduce calls the budget is actually spent on. The D78 rejection
/// checkpoint shares their prefix, so it is excluded rather than counted as a
/// second attempt.
fn shape_repair_attempts(host: &RecordingHost) -> Vec<String> {
    host.call_ids()
        .into_iter()
        .filter(|id| {
            id.starts_with("verification-repair-shape-repair-1-1-")
                && !id.ends_with("-semantic-preservation-rejected")
        })
        .collect()
}

fn shape_repair_responder(method: &str, id: &str) -> Value {
    if method == "checkpoint" {
        // The D78 rejection record. Counted, but it is the reduce calls the
        // budget is spent on.
        return json!({ "status": "accepted" });
    }
    assert!(
        id.starts_with("verification-repair-shape-repair-"),
        "unexpected host call: {method} {id}"
    );
    rejected_shape_repair()
}

#[tokio::test]
async fn an_identical_rejection_stops_the_loop_instead_of_spending_the_budget() {
    let host = RecordingHost::new(Box::new(shape_repair_responder));
    let driver = driver(host.clone(), 4);
    let mut evidence = LifecycleEvidence::default();

    driver
        .run_verification_shape_repair(
            repair_inventory(),
            &verification(),
            &[ALLOWED_TASK.to_string()],
            (1, 1),
            &mut evidence,
        )
        .await
        .expect("shape repair returns an inventory");

    assert_eq!(
        shape_repair_attempts(&host).len(),
        2,
        "the second identical rejection is the answer, not a reason for a third \
         attempt against unchanged input; calls: {:?}",
        host.call_ids()
    );
    let escalation = evidence
        .repair_attempts
        .iter()
        .find(|attempt| attempt["call_id"] == "verification-repair-shape-unsatisfiable-1-1")
        .expect("the repeated rejection is recorded as a route, not as silence");
    assert_eq!(
        escalation["issue_kind"],
        "verification_repair_shape_unsatisfiable"
    );
    assert_eq!(escalation["reason"], REPEATED_REJECTION_ROUTE_REASON);
}

#[tokio::test]
async fn a_rejection_that_changes_still_buys_another_attempt() {
    // The escape hatch is for a cause that reproduces. A reducer that breaks
    // something different each time is answering a different question every
    // round, so it keeps its whole budget: here it alternates between
    // reclassifying the item and dropping it outright.
    let host = RecordingHost::new(Box::new(|method: &str, id: &str| {
        if method == "checkpoint" {
            return json!({ "status": "accepted" });
        }
        let attempt = id
            .rsplit('-')
            .next()
            .and_then(|tail| tail.parse::<usize>().ok())
            .expect("shape repair call ids end in the attempt number");
        if attempt.is_multiple_of(2) {
            return json!({ "status": "accepted", "items": [], "unresolved_issues": [] });
        }
        let mut item = retry_item();
        item["classification"] = json!("environment_dependency_missing");
        json!({ "status": "accepted", "items": [item], "unresolved_issues": [] })
    }));
    let driver = driver(host.clone(), 3);
    let mut evidence = LifecycleEvidence::default();

    driver
        .run_verification_shape_repair(
            repair_inventory(),
            &verification(),
            &[ALLOWED_TASK.to_string()],
            (1, 1),
            &mut evidence,
        )
        .await
        .expect("shape repair returns an inventory");

    assert_eq!(
        shape_repair_attempts(&host).len(),
        3,
        "calls: {:?}",
        host.call_ids()
    );
    assert!(
        !evidence
            .repair_attempts
            .iter()
            .any(|attempt| attempt["issue_kind"] == "verification_repair_shape_unsatisfiable")
    );
}

#[test]
fn violations_are_compared_as_a_set_not_as_the_order_they_were_produced_in() {
    let one = ["dropped 'a'".to_string(), "mutated 'b'".to_string()];
    let reordered = ["mutated 'b'".to_string(), "dropped 'a'".to_string()];
    let different = ["dropped 'a'".to_string(), "mutated 'c'".to_string()];

    assert_eq!(violation_signature(&one), violation_signature(&reordered));
    assert_ne!(violation_signature(&one), violation_signature(&different));
}

#[test]
fn the_route_lets_the_guard_accept_a_reclassification_it_otherwise_refuses() {
    let original = [retry_item()];
    let mut reclassified = retry_item();
    reclassified["classification"] = json!("environment_dependency_missing");
    let refused = check_items(&original, std::slice::from_ref(&reclassified));
    assert!(!refused.passed());
    assert!(refused.violations[0].contains("classification"));

    let route = unsatisfiable_shape_route(&json!({ "items": [reclassified] }), &refused.violations);
    let rewritten = predicate_rewrite_inventory(&route, &verification())
        .expect("the escalation declares the route the rewriter consumes");
    let items = support::array(rewritten.get("items"));

    assert!(check_items(&original, &items).passed());
    assert_eq!(items[0]["source_residual_gap_ids"][0], "registry-schema-v2");
}

#[test]
fn the_route_still_refuses_to_let_gap_identity_move() {
    // Re-authoring is not a licence to forget what failed. A candidate that
    // drops the gap identity is rejected with the route declared exactly as it
    // is without it, so the escalation adopts nothing and the loop simply ends.
    let original = [retry_item()];
    let mut forgetful = retry_item();
    forgetful
        .as_object_mut()
        .expect("item object")
        .remove("source_residual_gap_ids");
    forgetful["source_item_id"] = json!("UNMATCHED-SOURCE");

    let route = unsatisfiable_shape_route(&json!({ "items": [forgetful] }), &[]);
    let rewritten = predicate_rewrite_inventory(&route, &verification()).expect("route");
    let items = support::array(rewritten.get("items"));

    let check = check_items(&original, &items);
    assert!(!check.passed());
    assert!(check.violations[0].contains("source_residual_gap_ids"));
}
