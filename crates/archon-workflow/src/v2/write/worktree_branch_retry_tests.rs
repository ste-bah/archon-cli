//! The in-run retry is told why the first session ended. A stall is not a
//! finished implementation waiting on one test run.

use super::super::super::errors::write_branch_interrupted_result;
use super::super::super::partial_work::{PartialOrigin, PartialWork, with_host_preamble};
use super::*;

fn input() -> serde_json::Value {
    serde_json::json!({"item": {"canonical_task_ids": ["TASK-001"]}})
}

/// The text the host dispatch hands the write layer for an inactivity cut:
/// typed as a host call timeout, the inactivity marker inside it.
fn stalled_error() -> String {
    crate::WorkflowError::HostCallTimeout(format!(
        "agent transport failed: {} no model output, tool call or tool result for 1800s",
        crate::error::INACTIVITY_TIMEOUT_MARKER
    ))
    .to_string()
}

fn wall_clock_error() -> String {
    crate::WorkflowError::HostCallTimeout(
        "agent transport failed: subagent timed out after 7200s".into(),
    )
    .to_string()
}

fn partial_from(result: &WorkflowV2Result) -> PartialWork {
    PartialWork {
        patch_path: "p".into(),
        files: vec!["a.rs".into()],
        bytes: 1,
        baseline_commit: "c".into(),
        origin: Some(PartialOrigin::from_result(result)),
    }
}

#[test]
fn an_inactivity_cut_is_recorded_as_a_stall_and_still_retried() {
    let result = write_branch_interrupted_result("b", &input(), &stalled_error());

    assert_eq!(result.data["branch_inactivity_timeout"], true);
    assert!(
        timed_out_with_work_unjudged(&result),
        "a stall with work on disk still earns the one in-run retry"
    );
    assert_eq!(retry_cause(&result), RetryCause::Stalled);
    assert!(result.summary.contains("stalled"), "{}", result.summary);
}

#[test]
fn the_retry_after_a_stall_is_not_told_its_tests_pass() {
    let result = write_branch_interrupted_result("b", &input(), &stalled_error());
    let instruction = retry_instruction(retry_cause(&result));

    assert!(!instruction.contains("believed to pass"), "{instruction}");
    assert!(instruction.contains("stalled"), "{instruction}");
    assert!(
        instruction.contains("Continue the implementation"),
        "{instruction}"
    );

    let text = with_host_preamble(
        "do the task",
        None,
        Some(&partial_from(&result)),
        &Default::default(),
    );
    assert!(!text.contains("ran out of time"), "{text}");
    assert!(text.contains("stalled"), "{text}");
}

#[test]
fn the_retry_after_a_wall_clock_cut_keeps_its_instruction() {
    let result = write_branch_interrupted_result("b", &input(), &wall_clock_error());

    assert_eq!(result.data["branch_inactivity_timeout"], false);
    assert_eq!(retry_cause(&result), RetryCause::WallClock);
    assert_eq!(retry_instruction(RetryCause::WallClock), RETRY_INSTRUCTION);
    let text = with_host_preamble(
        "do the task",
        None,
        Some(&partial_from(&result)),
        &Default::default(),
    );
    assert!(text.contains("ran out of time"), "{text}");
}

/// Issue-213 C2: the runner's no-progress stop keeps its work but is terminal
/// for the branch: it is never offered to the in-run retry, and its result
/// says it goes to remediation.
#[test]
fn a_no_progress_stop_is_kept_but_never_re_asked_in_run() {
    for detail in [
        "the last 8 tool calls returned fewer than 2 distinct results",
        "oscillation: the working tree returned to a state it had already left 3 times",
    ] {
        let error = format!(
            "agent transport failed: subagent failed: {} {detail}",
            crate::error::NO_PROGRESS_STOP_MARKER
        );
        assert!(super::super::super::errors::is_recoverable_write_branch_interruption(&error));
        let result = write_branch_interrupted_result("b", &input(), &error);
        assert_eq!(result.status, WorkflowV2Status::NeedsReview);
        assert_eq!(result.data["branch_no_progress_stop"], true);
        assert!(stopped_for_no_progress(&result));
        assert!(!timed_out_with_work_unjudged(&result), "{detail}");
        assert!(result.summary.contains("no progress"), "{}", result.summary);
        assert!(
            result.summary.contains("routed for remediation"),
            "{}",
            result.summary
        );
    }
}
