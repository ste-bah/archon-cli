// The third verdict, driven through the real review loop.

use serde_json::Value;

use super::LifecycleEvidence;
use super::review_test_host::{
    RecordingHost, Responder, accepted_report, driver, preamble_reply, seeded_review_evidence,
};

/// A finding that meets every admission condition: raised by a host-attributed
/// per-task review branch, naming its task, with a reason and a `file:line`.
fn admissible_finding() -> Value {
    serde_json::json!({
        "id": "finding-invalid-1",
        "verdict": "assignment_invalid",
        "canonical_task_ids": ["TASK-001"],
        "attribution_source": "review_branch",
        "reason": "the task requires a rate-limit hook on the provider trait, but the trait is \
                   sealed by the topology crate and has no interception seam",
        "evidence": ["crates/archon-llm/src/provider.rs:212 the trait is sealed here"],
        "severity": "blocking",
    })
}

fn responder() -> Responder {
    Box::new(|method, id| {
        if let Some(reply) = preamble_reply(id) {
            return reply;
        }
        if id.starts_with("cross-cutting-review-") {
            // The reduce ACCEPTS. The verdict has to survive a cross-cutting
            // round that saw nothing wrong, or the property under test would be
            // the reducer's opinion rather than the merge's.
            return serde_json::json!({ "status": "accepted", "items": [] });
        }
        if method == "finalReport" {
            return accepted_report();
        }
        panic!("unexpected host call: {method} {id}");
    })
}

#[tokio::test]
async fn assignment_invalid_ends_the_loop_after_one_round() {
    let host = RecordingHost::new(responder());
    let driver = driver(host.clone());
    let mut evidence = LifecycleEvidence::default();
    evidence
        .review
        .push(seeded_review_evidence(vec![admissible_finding()]));

    driver
        .run_review_and_final_gates(&serde_json::json!({ "items": [] }), &mut evidence)
        .await
        .expect("review gates run to a report");

    // The round count is the claim. A loop that treated the verdict as
    // remediable would have run seven cross-cutting rounds and six remediation
    // waves before reporting the same blocked status.
    assert_eq!(
        host.count_starting_with("cross-cutting-review-"),
        1,
        "calls: {:?}",
        host.call_ids()
    );
    assert_eq!(host.count_starting_with("review-remediation-inventory-"), 0);
    assert_eq!(host.count_starting_with("review-remediation-wave-"), 0);
    assert_eq!(
        host.ids_starting_with("blocked-assignment-invalid-"),
        vec!["blocked-assignment-invalid-1".to_string()]
    );
}

#[tokio::test]
async fn assignment_invalid_without_file_line_evidence_is_remediated_instead() {
    let mut finding = admissible_finding();
    finding["evidence"] = serde_json::json!(["the provider trait cannot be extended"]);
    let host = RecordingHost::new(Box::new(|method, id| {
        if let Some(reply) = preamble_reply(id) {
            return reply;
        }
        if id.starts_with("cross-cutting-review-") {
            return serde_json::json!({ "status": "accepted", "items": [] });
        }
        if id.starts_with("review-remediation-inventory-") {
            // Deliberately unrepairable: the test only needs to prove the
            // refused claim entered remediation at all, and the empty-inventory
            // block is the cheapest exit once it has.
            return serde_json::json!({ "status": "needs_review", "items": [] });
        }
        if method == "finalReport" {
            return accepted_report();
        }
        panic!("unexpected host call: {method} {id}");
    }));
    let driver = driver(host.clone());
    let mut evidence = LifecycleEvidence::default();
    evidence.review.push(seeded_review_evidence(vec![finding]));

    driver
        .run_review_and_final_gates(&serde_json::json!({ "items": [] }), &mut evidence)
        .await
        .expect("review gates run to a report");

    assert_eq!(host.count_starting_with("blocked-assignment-invalid-"), 0);
    assert_eq!(host.count_starting_with("review-remediation-inventory-"), 1);
}

#[tokio::test]
async fn a_reducer_cannot_claim_assignment_invalid() {
    // The cross-cutting reduce holds the whole run and is the one agent with an
    // incentive to declare a task it could not reconcile "invalid". Its items
    // are marked `cross_cutting` by the merge, which is what refuses them.
    let host = RecordingHost::new(Box::new(|method, id| {
        if let Some(reply) = preamble_reply(id) {
            return reply;
        }
        if id.starts_with("cross-cutting-review-") {
            return serde_json::json!({
                "status": "needs_remediation",
                "items": [{
                    "id": "cross-1",
                    "verdict": "assignment_invalid",
                    "canonical_task_ids": ["TASK-001"],
                    "attribution_source": "review_branch",
                    "reason": "the whole decomposition is wrong and none of these tasks \
                               should be attempted as written",
                    "evidence": ["crates/archon-llm/src/provider.rs:212 sealed trait"],
                }],
            });
        }
        if id.starts_with("review-remediation-inventory-") {
            return serde_json::json!({ "status": "needs_review", "items": [] });
        }
        if method == "finalReport" {
            return accepted_report();
        }
        panic!("unexpected host call: {method} {id}");
    }));
    let driver = driver(host.clone());
    let mut evidence = LifecycleEvidence::default();

    driver
        .run_review_and_final_gates(&serde_json::json!({ "items": [] }), &mut evidence)
        .await
        .expect("review gates run to a report");

    assert_eq!(host.count_starting_with("blocked-assignment-invalid-"), 0);
    assert_eq!(host.count_starting_with("review-remediation-inventory-"), 1);
}
