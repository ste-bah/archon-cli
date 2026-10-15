use anyhow::Error;

use crate::command::workflow_gate::{self, GateEvaluation, GateId};

pub(super) fn body_input_finding(error: &Error) -> Option<GateEvaluation> {
    workflow_gate::task_body_input_finding(error, GateId::WorkflowLintTaskSet, "task-set")
}

pub(super) fn gate_error_evaluation(error: &Error) -> GateEvaluation {
    // Unanswered fidelity batches remain resumable operational errors.
    if let Some((report, status)) = crate::command::topology_lint::resumable_exit(error) {
        eprintln!("{report}");
        crate::command::workflow_host_exit_drain::exit_after_drain(status);
    }
    crate::command::workflow_host_command_operational::exit_if_unsettled_publish(error);
    GateEvaluation::new("", Vec::new()).with_operational_error(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_task_parse_errors_become_body_findings_with_the_path_subject() {
        let error = Error::new(archon_workflow::WorkflowError::TaskBodyInvalid(
            "no fenced task block in /tmp/tasks/TASK-X-010.md".into(),
        ));
        let evaluation = body_input_finding(&error).expect("typed body error is a finding");
        let envelope = evaluation.into_envelope().unwrap();
        assert_eq!(envelope.policy_findings.len(), 1);
        assert_eq!(envelope.policy_findings[0].subject, "TASK-X-010");
        assert_eq!(
            envelope.policy_findings[0].remediation_scope,
            archon_workflow::RemediationScope::Body
        );
        assert!(envelope.operational_error.is_none());
    }

    #[test]
    fn unrelated_gate_errors_remain_operational() {
        let error = Error::new(archon_workflow::WorkflowError::SpecInvalid(
            "gate setup failed".into(),
        ));
        assert!(body_input_finding(&error).is_none());
    }
}
