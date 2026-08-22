//! Authoring-attempt budgeting.
//!
//! These pin the distinction that killed run wf-ac47347c: its first authoring
//! attempt was rejected for a one-line missing `export const meta` marker, its
//! second was CANCELLED 33 seconds in without producing a script at all, and
//! the run ended there — a transport blip spending the only retry reserved for
//! fixing an actual defect.

use super::{
    MAX_AUTHORING_DEFECT_ATTEMPTS, MAX_AUTHORING_TRANSPORT_ATTEMPTS, is_transport_failure,
};
use archon_workflow::WorkflowError;

/// The exact string the subagent layer produced live, before `describe_join_error`
/// stopped calling a cancellation a panic. Both spellings must classify as
/// transport, so a deployed binary and an older recorded failure agree.
#[test]
fn a_cancelled_authoring_call_is_a_transport_failure() {
    for text in [
        "agent transport failed: workflow stage failed: subagent failed: join panic: task 431 was cancelled",
        "agent transport failed: subagent cancelled before returning a result: task 431 was cancelled",
        "subagent failed: request timed out",
    ] {
        assert!(
            is_transport_failure(&WorkflowError::SpecInvalid(text.to_string())),
            "must be transport, not a defect: {text}"
        );
    }
}

/// A rejected script IS the author's fault and must consume a defect attempt —
/// otherwise a model that never satisfies the pre-flight loops forever.
#[test]
fn a_rejected_script_is_not_a_transport_failure() {
    for text in [
        "authored workflow.js is missing the required `export const meta` declaration",
        "these task ids have NO write coverage: TASK-A-001",
        "the script plans ZERO agent calls across 3 host call(s)",
    ] {
        assert!(
            !is_transport_failure(&WorkflowError::SpecInvalid(text.to_string())),
            "must be a defect, not transport: {text}"
        );
    }
}

/// Both budgets must exceed the old behaviour (one attempt plus one retry),
/// and the transport budget must be independent — a run whose only retry is
/// eaten by a cancellation never gets to fix the defect it was told about.
#[test]
fn authoring_budgets_leave_room_to_actually_repair() {
    assert!(
        MAX_AUTHORING_DEFECT_ATTEMPTS > 2,
        "two attempts is what failed live"
    );
    assert!(
        MAX_AUTHORING_TRANSPORT_ATTEMPTS >= 2,
        "a single transport failure must not end the run"
    );
}
