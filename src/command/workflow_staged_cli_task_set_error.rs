use anyhow::Error;

use crate::command::workflow_gate::{self, GateEvaluation, GateId};

pub(super) fn body_input_finding(error: &Error) -> Option<GateEvaluation> {
    workflow_gate::task_body_input_finding(error, GateId::WorkflowLintTaskSet, "task-set")
}

pub(crate) fn gate_error_evaluation(error: &Error) -> GateEvaluation {
    if let Some(incomplete) = crate::command::topology_lint::LintIncomplete::caused(error) {
        eprintln!("{}", incomplete.report());
    }
    crate::command::workflow_host_command_operational::exit_if_unsettled_publish(error);
    let evaluation = crate::command::topology_lint::LintIncomplete::caused(error)
        .and_then(|incomplete| incomplete.evaluation.clone())
        .unwrap_or_else(|| GateEvaluation::new("", Vec::new()))
        .with_host_error(error);
    if crate::command::workflow_freeze_budget::FreezeIncomplete::caused(error).is_some_and(
        crate::command::workflow_freeze_budget::FreezeIncomplete::is_authored_judge_reply,
    ) {
        evaluation.with_operational_error_kind("authored_judge_reply")
    } else {
        evaluation
    }
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

    #[test]
    fn authored_incomplete_freeze_judge_keeps_its_kind_in_the_gate_envelope() {
        let progress = crate::command::workflow_freeze_budget::FreezeProgress::default();
        let error = crate::command::workflow_freeze_budget::FreezeIncomplete::stalled(
            "malformed judge reply",
            &progress,
        )
        .authored_judge_reply();
        let envelope = gate_error_evaluation(&Error::new(error))
            .into_envelope()
            .unwrap();
        assert_eq!(
            envelope.operational_error.unwrap().kind,
            "authored_judge_reply"
        );
    }

    #[test]
    fn typed_provider_transport_keeps_the_transient_operational_kind() {
        let envelope = gate_error_evaluation(&Error::new(std::io::Error::from(
            std::io::ErrorKind::ConnectionReset,
        )))
        .into_envelope()
        .unwrap();
        assert_eq!(
            envelope.operational_error.unwrap().kind,
            "transient_host_fault"
        );
    }
    #[test]
    fn host_command_typed_decode_fault_preserves_path_and_remedy() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        ))
        .context("reading TASK file TASK-X-001.md");
        let error = gate_error_evaluation(&error)
            .into_envelope()
            .unwrap()
            .operational_error
            .unwrap();
        assert!(error.text.contains("TASK-X-001.md"));
        assert!(
            error.text.contains("stream did not contain valid UTF-8"),
            "{}",
            error.text
        );
        assert_eq!(error.kind, "permanent_host_fault");
        assert!(error.text.contains("restore valid bytes"), "{}", error.text);
    }
}
