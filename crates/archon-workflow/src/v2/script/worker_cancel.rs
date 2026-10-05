//! Issue 335: only run control cancels a run.
//!
//! An operator cancel reaches a call as a control error and the run as its
//! stored `Cancelled` state; it never travels as a result status. A
//! `cancelled` status in a call's result is the worker's own report (or a
//! fanout counting its branches' reports): the work did not finish. Taken
//! as it was, that one report cancelled the whole run -- a terminal host
//! stop with the highest status precedence. It is a failed call instead:
//! the script's remediation handles it like any other failure, and the
//! worker's own report is kept in the summary.

use super::*;

/// The `data` key that records the status the worker itself reported.
const WORKER_REPORTED_STATUS_KEY: &str = "worker_reported_status";

/// Turns a worker's own `cancelled` result for `call_id` into a failed
/// call; any other result is left as it is.
pub(super) fn worker_cancel_as_failed_call(call_id: &str, result: &mut WorkflowV2Result) {
    if result.status != WorkflowV2Status::Cancelled {
        return;
    }
    let reported = match result.summary.trim() {
        "" => "no summary",
        summary => summary,
    };
    result.status = WorkflowV2Status::Failed;
    result.summary = format!(
        "call '{call_id}' reported status 'cancelled' in its own result; only run control cancels a run, so it is a failed call for remediation: {reported}"
    );
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("worker_reported_cancelled_{}", sanitize_v2_gap_id(call_id)),
        description: "the worker stopped and reported 'cancelled' without finishing the work; remediate or run the call again".to_string(),
        severity: Some("blocking".to_string()),
    });
    let reported_status = serde_json::Value::from("cancelled");
    if result.data.is_null() {
        result.data = serde_json::json!({});
    }
    // Data of another shape is the worker's; the summary names the report.
    if let Some(object) = result.data.as_object_mut() {
        object.insert(WORKER_REPORTED_STATUS_KEY.to_string(), reported_status);
    }
}
