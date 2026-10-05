//! Issue 335: a worker's own 'cancelled' result is a failed call the script
//! remediates, never a run cancel; only run control cancels a run.

use super::*;

fn agent_call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: Default::default(),
    }
}

fn cancelled(summary: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Cancelled,
        summary: summary.to_string(),
        ..WorkflowV2Result::default()
    }
}

#[test]
fn a_worker_reporting_cancelled_is_a_failed_call_with_its_report() {
    let execution = WorkflowV2CallExecution {
        call: agent_call("implement-task-a"),
        input: serde_json::Value::Null,
        depends_on: Vec::new(),
    };
    let result = normalize_result_for_call(&execution, cancelled("stopped before the edit"));
    assert_eq!(result.status, WorkflowV2Status::Failed, "{result:?}");
    assert!(
        result.summary.contains("stopped before the edit"),
        "the worker's own report is kept: {}",
        result.summary
    );
    assert!(
        result.summary.contains("only run control cancels a run"),
        "{}",
        result.summary
    );
    assert_eq!(result.data["worker_reported_status"], "cancelled");
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with("worker_reported_cancelled_")),
        "a remediable gap names it: {:?}",
        result.residual_gaps
    );
    assert!(result.validate().is_ok(), "{:?}", result.validate());
}

#[test]
fn a_worker_result_that_is_not_cancelled_is_unchanged() {
    let execution = WorkflowV2CallExecution {
        call: agent_call("inspect"),
        input: serde_json::Value::Null,
        depends_on: Vec::new(),
    };
    let mut failed = cancelled("could not read the file");
    failed.status = WorkflowV2Status::Failed;
    let result = normalize_result_for_call(&execution, failed.clone());
    assert_eq!(result.status, WorkflowV2Status::Failed);
    assert_eq!(result.summary, failed.summary);
    assert!(result.residual_gaps.is_empty());
}

#[test]
fn a_cancelled_call_status_is_not_a_terminal_host_stop() {
    assert!(!terminal_stop_for_call(
        &agent_call("review-1"),
        WorkflowV2Status::Cancelled
    ));
    // The gates keep their rule.
    let mut gate = agent_call("approval");
    gate.method = WorkflowV2HostMethod::HumanGate;
    assert!(terminal_stop_for_call(&gate, WorkflowV2Status::NeedsReview));
    assert!(!terminal_stop_for_call(&gate, WorkflowV2Status::Accepted));
}
