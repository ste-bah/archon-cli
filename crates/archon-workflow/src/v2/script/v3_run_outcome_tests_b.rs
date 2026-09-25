//! Second hostile review of the terminal rule: each test failed on 8da643191.

use super::*;
use WorkflowV2Status::{Accepted, Failed, NeedsReview};

fn unassigned(severity: serde_json::Value) -> Case {
    let mut case = Case::clean();
    let mut finding =
        serde_json::json!({ "claim": "a set-level note", "attributable_to_task": false });
    if !severity.is_null() {
        finding["severity"] = severity;
    }
    case.result = accounting(serde_json::json!({
        "adversarial_findings": [finding],
        "review_remediation": { "resolved": [], "unresolved": [], "unassigned": [] },
    }));
    case
}

// 1: block unless the severity is explicitly low-impact.
#[test]
fn an_unassigned_finding_blocks_unless_its_severity_is_on_the_allow_list() {
    for severity in [
        serde_json::json!("medium"),
        serde_json::json!("MAJOR"),
        serde_json::Value::Null,
        serde_json::json!(3),
    ] {
        unassigned(severity.clone()).holds("names no task");
    }
    for severity in [
        "low",
        " Info ",
        "informational",
        "note",
        "minor",
        "trivial",
        "nit",
    ] {
        let outcome = unassigned(serde_json::json!(severity)).decide();
        assert_eq!(
            outcome.status,
            Accepted,
            "{severity}: {}",
            outcome.explanation()
        );
        assert!(
            outcome.explanation().contains("a set-level note"),
            "{}",
            outcome.explanation()
        );
    }
}

// 2: only a universe task without writable files may be not_task_actionable.
#[test]
fn not_task_actionable_for_an_id_outside_the_universe_blocks() {
    let mut case = Case::clean();
    case.writable.remove(A);
    case.result = accounting(serde_json::json!({
        "review_remediation": { "resolved": [], "unresolved": [
            { "taskId": "TASK-ZZZ", "outcome": "not_task_actionable" },
            { "taskId": " task-a ", "outcome": "not_task_actionable" },
        ], "unassigned": [] },
    }));
    // " task-a " is TASK-A (declares nothing writable here); TASK-ZZZ is no
    // task at all, so its claim cannot stand.
    let outcome = case.decide();
    assert_eq!(outcome.blocking.len(), 1, "{}", outcome.explanation());
    assert!(
        outcome.blocking[0].contains("TASK-ZZZ"),
        "{}",
        outcome.explanation()
    );
}

// 3: remediation the acceptance stage dispatched must be backed too.
#[test]
fn acceptance_stage_remediation_needs_an_accepted_fix_and_verifier() {
    let mut case = Case::clean();
    case.calls.push(fix(B, 1, Accepted));
    case.calls.push(rverify(B, 1, NeedsReview, true));
    case.holds("acceptance-stage remediation of TASK-B");
}

// 8: a reviewer verdict is a review; only an execution or contract failure,
// or a missing result, means no review happened.
#[test]
fn a_reviewer_that_returned_failed_still_reviewed() {
    let mut case = Case::clean();
    let mut map = case.calls[3].clone();
    map.tasks.insert(
        B.into(),
        AuthoredTaskOutcome {
            status: Failed,
            transport: false,
            not_reviewed: false,
        },
    );
    case.calls[3] = map;
    let outcome = case.decide();
    assert_eq!(outcome.status, Accepted, "{}", outcome.explanation());
    let mut case = Case::clean();
    case.calls[3].tasks.insert(
        B.into(),
        AuthoredTaskOutcome {
            status: Failed,
            transport: false,
            not_reviewed: true,
        },
    );
    case.holds("did not review task TASK-B");
}

// NEW: a finding that names tasks but no single task may fix is resolved by
// the cross-task remediation over all of them, and by nothing else.
#[test]
fn a_cross_task_finding_is_resolved_only_by_a_backed_cross_task_remediation() {
    let key = "cross:TASK-A+TASK-B";
    let cross = |stage_fix: bool, status: WorkflowV2Status, b_status: WorkflowV2Status| {
        let role = if stage_fix {
            AuthoredCallRole::RemediationFix {
                task: key.into(),
                round: 1,
            }
        } else {
            AuthoredCallRole::RemediationVerify {
                task: key.into(),
                round: 1,
                agent: true,
            }
        };
        fact(
            &format!("{key}-{stage_fix}"),
            role,
            status,
            &[(A, status), (B, b_status)],
        )
    };
    let finding = serde_json::json!([
        { "id": "f1", "canonical_task_ids": [A], "severity": "high" },
        { "claim": "the chain between A and B is broken", "attributable_to_task": false, "canonical_task_ids": ["task-b", A], "severity": "high" },
    ]);
    let with = |resolved: serde_json::Value| {
        let mut case = Case::clean();
        case.result = accounting(serde_json::json!({
            "adversarial_findings": finding,
            "review_remediation": { "resolved": resolved, "unresolved": [], "unassigned": [] },
        }));
        case
    };
    with(serde_json::json!([{ "taskId": A }]))
        .holds("review findings name task cross:TASK-A+TASK-B");
    let mut backed = with(
        serde_json::json!([{ "taskId": A }, { "taskId": "cross:TASK-B+TASK-A", "crossTask": true }]),
    );
    backed.calls.insert(9, cross(true, Accepted, Accepted));
    backed.calls.insert(10, cross(false, Accepted, Accepted));
    assert_eq!(
        backed.decide().status,
        Accepted,
        "{}",
        backed.decide().explanation()
    );
    // The verifier must pass EVERY named task.
    backed.calls[10] = cross(false, Accepted, NeedsReview);
    backed.holds("is NeedsReview for TASK-B");
}
