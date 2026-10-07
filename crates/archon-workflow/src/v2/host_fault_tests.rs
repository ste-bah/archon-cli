//! A call that never started must not read as an attempt by the work.

use super::*;
use crate::v2::WorkflowV2Status;

fn marker_present(result: &WorkflowV2Result, marker: &str) -> bool {
    result
        .data
        .get(marker)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// The live shape: the host could not read a file in its own run store, so no
/// branch was dispatched and nothing judged the work. The result is still a
/// failure — the stage did not succeed — but it carries the never-ran markers.
#[test]
fn a_call_that_never_started_is_marked_as_carrying_no_verdict() {
    let error = WorkflowError::StateCorrupt("results/record.json: missing field".to_string());
    let result = v2_result_for_call_error("stage-1", &error);

    assert!(is_never_started_fault(&error));
    assert_eq!(result.status, WorkflowV2Status::Failed);
    assert!(marker_present(&result, HOST_FAULT_NO_VERDICT_MARKER));
    assert!(marker_present(&result, NO_VERDICT_REFUND_MARKER));
}

/// The other half of the contract: an attempt that ran and failed is charged.
/// Nothing here may make a genuine failure look free, or a task that really
/// cannot be finished would loop instead of being recorded blocked.
#[test]
fn an_attempt_that_ran_and_failed_carries_no_never_started_marker() {
    for error in [
        WorkflowError::StageFailed("the verifier rejected the change".to_string()),
        WorkflowError::StageBlocked("dependency unmet".to_string()),
        WorkflowError::HostCallTimeout("dispatch timer ended the session".to_string()),
        WorkflowError::PolicyDenied("write outside declared targets".to_string()),
    ] {
        assert!(!is_never_started_fault(&error), "{error}");
        let result = v2_result_for_call_error("stage-1", &error);
        assert_eq!(result.status, WorkflowV2Status::Failed);
        assert!(
            !marker_present(&result, HOST_FAULT_NO_VERDICT_MARKER),
            "{error}"
        );
        assert!(
            !marker_present(&result, NO_VERDICT_REFUND_MARKER),
            "{error}"
        );
    }
}

/// The containment bound. A never-ran failure returns instantly, so without
/// this the loop that owns the retry runs its whole budget in milliseconds and
/// then starts on the next task. The second consecutive one stops the run.
#[test]
fn a_second_consecutive_never_started_call_refuses_to_continue() {
    let mut streak = NeverStartedStreak::default();

    assert!(
        !streak.record_never_started(),
        "the first is a failed stage"
    );
    assert!(
        streak.record_never_started(),
        "the second must stop the run"
    );
}

/// A call that executed means the host is working again, whatever the call
/// decided, so the streak starts over rather than accumulating across a run.
#[test]
fn an_executed_call_resets_the_streak() {
    let mut streak = NeverStartedStreak::default();

    assert!(!streak.record_never_started());
    streak.record_executed();
    assert_eq!(streak.consecutive(), 0);
    assert!(!streak.record_never_started(), "the streak restarted");
}

/// The bound must hold below any remediation budget a script can ask for: the
/// point is that a bounded loop cannot complete its whole budget with nothing
/// executed. Six is the standing default attempt count.
#[test]
fn the_bound_stops_a_run_well_inside_a_bounded_remediation_budget() {
    let mut streak = NeverStartedStreak::default();
    let mut instant_failures = 0usize;

    for _ in 0..6 {
        instant_failures += 1;
        if streak.record_never_started() {
            break;
        }
    }

    assert!(
        instant_failures < 6,
        "a budget of six was spent entirely on calls that never ran"
    );
    assert_eq!(instant_failures, NeverStartedStreak::DEFAULT_LIMIT);
}

/// A caller cannot disable the bound by asking for zero.
#[test]
fn a_zero_limit_still_stops_on_the_first_never_started_call() {
    let mut streak = NeverStartedStreak::new(0);

    assert!(streak.record_never_started());
}

/// The record an older binary (8b7a3c13f) wrote on a live run for an author
/// call whose provider stream gave up: unmarked. Copied from that run, with
/// the task prompt and the session id removed.
const OLD_BINARY_RECORD: &str =
    include_str!("../../tests/fixtures/v2_old_binary_dispatch_error_record.json");

fn old_binary_record() -> crate::v2::WorkflowV2CallRecord {
    serde_json::from_str(OLD_BINARY_RECORD).expect("the old record still deserializes")
}

/// Issue 337 round 5 (review findings 4 and 6): the old unmarked dispatch
/// error is recognised by its structure, rebuilt from the record's own call
/// id and compared whole, on the real record such a binary wrote.
#[test]
fn an_old_binary_dispatch_error_record_carries_no_verdict() {
    let record = old_binary_record();
    assert_eq!(record.result.status, WorkflowV2Status::Failed);
    assert!(result_carries_no_verdict(&record.call.id, &record.result));
}

/// Text that only looks like it is a verdict: the same result filed under
/// another call, or a result that differs from the producer's in any part.
#[test]
fn a_result_that_only_reads_like_an_old_dispatch_error_keeps_its_verdict() {
    let record = old_binary_record();
    assert!(
        !result_carries_no_verdict("acceptance-author-other", &record.result),
        "another call's text"
    );
    let mut gap = record.result.clone();
    gap.residual_gaps[0].severity = Some("high".into());
    assert!(!result_carries_no_verdict(&record.call.id, &gap), "a gap");
    let mut evidence = record.result.clone();
    evidence.evidence.clear();
    assert!(
        !result_carries_no_verdict(&record.call.id, &evidence),
        "no evidence"
    );
    let mut data = record.result.clone();
    data.data = serde_json::json!({ "error": "another error" });
    assert!(!result_carries_no_verdict(&record.call.id, &data), "data");
    let mut status = record.result.clone();
    status.status = WorkflowV2Status::NeedsReview;
    assert!(
        !result_carries_no_verdict(&record.call.id, &status),
        "status"
    );
}

/// What this build writes: a dispatch error is marked, an invalid answer
/// is marked as the answer it is.
#[test]
fn this_builds_failed_results_are_classified_by_their_marker() {
    let error = WorkflowError::StageFailed("agent transport failed".into());
    let fault = v2_result_for_call_error("author-1", &error);
    assert!(result_carries_no_verdict("author-1", &fault));
    let invalid = invalid_answer_result(
        "author-1",
        WorkflowError::SpecInvalid("answer has no status".into()),
    );
    assert!(!result_carries_no_verdict("author-1", &invalid));
}
