// A failed verification with a recorded gap is a request for work.
//
// The route to a write branch is gated on three opt-in markers the verifier
// must set. A verifier that simply reports the defect sets none of them, and
// the gate then reads silence as "nothing to act on". Observed live:
// TASK-TDL-020 failed five branches with precise findings — "implement all 17
// exact stable validation IDs", "add the seven missing focused test functions"
// — carrying no marker, so `verification-failure-triage` never ran once in the
// entire run and the task sat at 9 of 11 artifacts while the loop re-planned
// and re-verified an unchanged tree.

use serde_json::json;

use super::verify::failed_with_residual_gaps;

/// The live shape: failed, gaps recorded, no marker field anywhere.
#[test]
fn a_failed_outcome_with_gaps_is_actionable_without_any_marker() {
    let outcome = json!({
        "item_id": "verify-TASK-TDL-020-02-artifact-contract",
        "result": {
            "status": "failed",
            "summary": "artifact contract not satisfied",
            "residual_gaps": [
                { "description": "Implement and emit all 17 exact stable validation IDs." }
            ],
        },
    });
    assert!(failed_with_residual_gaps(&outcome));
}

/// A failure a writer cannot act on — a transport death names no defect — must
/// not spend a worktree.
#[test]
fn a_failure_without_gaps_is_not_actionable() {
    let outcome = json!({
        "result": { "status": "failed", "summary": "agent transport failed", "residual_gaps": [] },
    });
    assert!(!failed_with_residual_gaps(&outcome));
}

/// Accepted work is never dragged into remediation, gaps or not.
#[test]
fn an_accepted_outcome_is_never_actionable() {
    let outcome = json!({
        "result": {
            "status": "accepted",
            "residual_gaps": [{ "description": "a note about future work" }],
        },
    });
    assert!(!failed_with_residual_gaps(&outcome));
}

/// needs_review and blocked carry findings too, and both are reached by the
/// same loop.
#[test]
fn needs_review_and_blocked_with_gaps_are_actionable() {
    for status in ["needs_review", "blocked"] {
        let outcome = json!({
            "result": {
                "status": status,
                "residual_gaps": [{ "description": "add the seven missing focused test functions" }],
            },
        });
        assert!(failed_with_residual_gaps(&outcome), "status {status}");
    }
}

/// The envelope is read at either depth, since outcomes arrive both ways.
#[test]
fn a_flat_outcome_envelope_is_read_the_same_way() {
    let outcome = json!({
        "status": "failed",
        "residual_gaps": [{ "description": "no per-dataset-version validation.json was found" }],
    });
    assert!(failed_with_residual_gaps(&outcome));
}
