//! REM-14: the terminal rule proves every universe task from the host's
//! records, whatever the script's accounting lists.

use super::*;

const C: &str = "TASK-C";

fn with_c(case: &mut Case) {
    case.universe.insert(C.to_string());
    case.writable.insert(C.to_string());
}

fn c_write(id: &str, status: WorkflowV2Status) -> AuthoredCallFact {
    fact(id, AuthoredCallRole::Write, status, &[(C, status)])
}

fn c_verify(id: &str, status: WorkflowV2Status) -> AuthoredCallFact {
    fact(id, AuthoredCallRole::TaskVerify, status, &[(C, status)])
}

/// Where the reviews begin in `Case::clean`'s calls.
fn review_at(case: &Case) -> usize {
    case.calls
        .iter()
        .position(|call| matches!(call.role, AuthoredCallRole::Review { .. }))
        .expect("a review call")
}

#[test]
fn a_universe_task_the_script_never_dispatched_holds_the_run_by_name() {
    let mut case = Case::clean();
    with_c(&mut case);
    case.holds(
        "task TASK-C of the task universe reached no accepted outcome: the script's accounting does not name it and no host write record names it",
    );
}

#[test]
fn a_task_the_accounting_omits_holds_the_run_even_when_the_host_records_prove_it() {
    let mut case = Case::clean();
    with_c(&mut case);
    let at = review_at(&case);
    case.calls
        .insert(at, c_verify("verification-wave-c-verify-1", Accepted));
    case.calls.insert(at, c_write("c-impl-1", Noop));
    // The prelude folds every completed task into the accounting: one
    // still missing is an accounting the run cannot stand on.
    case.holds(
        "task TASK-C of the task universe is missing from the script's accounting, though the host's records show it accepted",
    );
    // Named, the same records accept it.
    case.result = accounting(serde_json::json!({ "accepted": [A, B, C] }));
    let outcome = case.decide();
    assert_eq!(outcome.status, Accepted, "{}", outcome.explanation());
}

#[test]
fn the_accounting_claiming_a_task_accepted_without_host_records_does_not_pass() {
    let mut case = Case::clean();
    with_c(&mut case);
    case.result = accounting(serde_json::json!({ "accepted": [A, B, C] }));
    case.holds("task TASK-C is reported accepted but no host write record names it");
    // Named once: the claim's own clause is the one that holds it.
    let outcome = case.decide();
    let about_c = outcome
        .blocking
        .iter()
        .filter(|clause| clause.contains(C))
        .count();
    assert_eq!(about_c, 1, "{}", outcome.explanation());
}

#[test]
fn a_write_after_the_reviews_began_is_no_accepted_outcome() {
    let mut case = Case::clean();
    with_c(&mut case);
    let at = review_at(&case) + 1;
    case.calls.insert(at, c_verify("verify-c-late", Accepted));
    case.calls.insert(at, c_write("write-c-late", Accepted));
    case.holds("task TASK-C of the task universe reached no accepted outcome");
}

#[test]
fn a_refused_verify_or_an_unverified_write_holds_the_task() {
    let mut refused = Case::clean();
    with_c(&mut refused);
    let at = review_at(&refused);
    refused.calls.insert(at, c_verify("verify-c", Failed));
    refused.calls.insert(at, c_write("write-c", Accepted));
    refused.holds("its latest verify `verify-c` is Failed");

    let mut unverified = Case::clean();
    with_c(&mut unverified);
    let at = review_at(&unverified);
    unverified.calls.insert(at, c_write("write-c", Accepted));
    unverified.holds("no host verify record names it");

    // A verify BEFORE its latest write judged some other tree.
    let mut stale = Case::clean();
    with_c(&mut stale);
    let at = review_at(&stale);
    stale.calls.insert(at, c_write("write-c-2", Accepted));
    stale.calls.insert(at, c_verify("verify-c-1", Accepted));
    stale.calls.insert(at, c_write("write-c-1", Accepted));
    stale.holds("nothing verified it after its latest write `write-c-2`");
}

#[test]
fn a_task_the_script_reports_blocked_is_judged_by_the_closure_alone() {
    let mut case = Case::clean();
    with_c(&mut case);
    case.result = accounting(serde_json::json!({
        "blocked": [{ "taskId": C, "reason": "budget exhausted" }],
    }));
    let outcome = case.decide();
    assert_eq!(outcome.status, NeedsReview, "{}", outcome.explanation());
    let about_c: Vec<&String> = outcome
        .blocking
        .iter()
        .filter(|clause| clause.contains(C))
        .collect();
    assert_eq!(about_c.len(), 1, "{}", outcome.explanation());
    assert!(
        about_c[0].contains("task TASK-C is blocked and review remediation closed no finding"),
        "{}",
        outcome.explanation()
    );
}

#[test]
fn a_task_that_declares_no_file_is_accepted_only_on_a_recorded_noop_verify() {
    let mut case = Case::clean();
    case.universe.insert(C.to_string());
    case.result = accounting(serde_json::json!({ "accepted": [A, B, C] }));
    let at = review_at(&case);
    case.calls.insert(at, c_verify("verify-c", Noop));
    let outcome = case.decide();
    assert_eq!(outcome.status, Accepted, "{}", outcome.explanation());
    // An accepted verify is not the verifier saying nothing needed to change.
    case.replace("verify-c", c_verify("verify-c", Accepted));
    case.holds("which the host accepts only as a recorded no-op verify");
    // A task that declares files it may write needs its write, noop or not.
    case.writable.insert(C.to_string());
    case.replace("verify-c", c_verify("verify-c", Noop));
    case.holds("task TASK-C is reported accepted but no host write record names it");
}

fn c_map(id: &str, kind: &str) -> AuthoredCallFact {
    let role = AuthoredCallRole::Review {
        kind: kind.into(),
        stage: "map".into(),
    };
    fact(id, role, Accepted, &[(C, Accepted)])
}

/// TASK-C blocked, then finished by review remediation (its fix and an
/// accepted verifier agent close the finding standing for it).
fn finished_c() -> Case {
    let mut case = Case::clean();
    with_c(&mut case);
    case.result = accounting(serde_json::json!({
        "blocked": [{ "taskId": C, "reason": "verifier refused" }],
    }));
    case.calls.insert(9, fix(C, 1, Accepted));
    case.calls.insert(10, rverify(C, 1, Accepted, true));
    case
}

#[test]
fn a_finished_blocked_task_stands_only_on_a_later_review_of_both_kinds() {
    let mut case = finished_c();
    case.holds("task TASK-C was finished by review remediation, but no later adversarial_findings review map reviewed it");
    // One kind is not both.
    case.calls.insert(
        11,
        c_map(
            "adversarial-review-moved-1-map",
            "adversarial_findings_moved",
        ),
    );
    case.holds("no later uncovered_requirements review map reviewed it");
    case.calls.insert(
        12,
        c_map("coverage-audit-moved-1-map", "uncovered_requirements_moved"),
    );
    let outcome = case.decide();
    assert_eq!(outcome.status, Accepted, "{}", outcome.explanation());
}

#[test]
fn a_review_before_the_finishing_verifier_or_one_that_did_not_review_does_not_count() {
    // Both maps BEFORE the verifier that finished it.
    let mut early = finished_c();
    early.calls.insert(
        9,
        c_map(
            "adversarial-review-moved-1-map",
            "adversarial_findings_moved",
        ),
    );
    early.calls.insert(
        9,
        c_map("coverage-audit-moved-1-map", "uncovered_requirements_moved"),
    );
    early.holds("task TASK-C was finished by review remediation, but no later");
    // After it, but the branch for TASK-C produced no verdict.
    let mut silent = finished_c();
    let mut map = c_map(
        "adversarial-review-moved-1-map",
        "adversarial_findings_moved",
    );
    map.tasks.get_mut(C).unwrap().not_reviewed = true;
    silent.calls.insert(11, map);
    silent.calls.insert(
        12,
        c_map("coverage-audit-moved-1-map", "uncovered_requirements_moved"),
    );
    let outcome = silent.decide();
    assert!(
        outcome
            .blocking
            .iter()
            .any(|clause| clause.contains("no later adversarial_findings review map reviewed it")),
        "{}",
        outcome.explanation()
    );
}
