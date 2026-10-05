//! Issue 293: a run that stopped is never a pass, and a script that never
//! returned its accounting is "no result", never a spec defect.

use super::*;
use WorkflowV2Status::{Accepted, NeedsReview, Noop};

fn stopped(
    accumulated: WorkflowV2Status,
    failed: Option<&str>,
    result: Option<&str>,
) -> AuthoredRunOutcome {
    authored_run_terminal_status(&AuthoredRunFacts {
        accumulated_status: accumulated,
        host_terminal_failure: failed,
        script_result: result,
        acceptance_gate: AuthoredAcceptanceGateFact::Missing,
        calls: &[],
        writable_tasks: &BTreeSet::new(),
        universe_tasks: &BTreeSet::new(),
    })
}

#[test]
fn a_hard_stop_over_a_passing_accumulator_is_incomplete_not_accepted() {
    let returned = accounting(serde_json::json!({}));
    for accumulated in [Accepted, Noop] {
        for (failed, result) in [
            (Some("approval-gate"), None),
            (Some("approval-gate"), Some(returned.as_str())),
            (None, None),
        ] {
            let outcome = stopped(accumulated, failed, result);
            assert_eq!(outcome.status, NeedsReview, "{}", outcome.explanation());
            assert!(!outcome.from_accounting);
        }
    }
}

#[test]
fn missing_accounting_is_no_result_not_a_spec_defect() {
    let expected = [A.to_string(), B.to_string()].into_iter().collect();
    assert!(crate::v2::script::validate_authored_task_accounting(None, &expected).is_ok());
    // A returned result is still judged in full.
    assert!(crate::v2::script::validate_authored_task_accounting(Some("{}"), &expected).is_err());
}
