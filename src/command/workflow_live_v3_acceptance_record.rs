//! One ran check as the round records it, split from
//! `workflow_live_v3_acceptance` for size.

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::AcceptanceCriterion;
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::acceptance_stage::{
    AcceptanceCheckRecordV1, AcceptanceCheckStatus, coverage,
};

use super::exec;
use super::output::tail;

pub(super) fn check_record(
    criterion: &AcceptanceCriterion,
    result: &CheckResult,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> AcceptanceCheckRecordV1 {
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    // A3: exit 0 from a run that ran no test at all proves nothing.
    let zero_work = result.operational_error.is_none()
        && result.exit_code == Some(0)
        && archon_workflow::acceptance::output_reports_zero_work(&stdout, &stderr);
    let status = if result.operational_error.is_some() {
        AcceptanceCheckStatus::Error
    } else if result.exit_code == Some(0) && !zero_work {
        AcceptanceCheckStatus::Passed
    } else {
        AcceptanceCheckStatus::Failed
    };
    AcceptanceCheckRecordV1 {
        check_id: criterion.id.clone(),
        criterion: criterion.criterion.clone(),
        kind: exec::check_kind(criterion).to_string(),
        status,
        exit_code: result.exit_code,
        operational_error: result.operational_error.clone(),
        owning_tasks: coverage::implementing_tasks(universe, &criterion.id, &criterion.covers),
        stdout_tail: if zero_work {
            format!(
                "[host] the check exited 0 but its output reports that no test ran, so it is counted as failed: whatever it names no longer exists or no longer matches\n{}",
                tail(&result.stdout)
            )
        } else {
            tail(&result.stdout)
        },
        stderr_tail: tail(&result.stderr),
        regressed_by: None,
        contract_defect: false,
        routing: None,
        regression_search: None,
        blocked: None,
    }
}
