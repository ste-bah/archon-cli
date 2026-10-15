use archon_workflow::HostCommandResult;

use crate::command::workflow_gate::{self, GateId};

/// Classify a gate without turning an operational failure into remediation.
/// Preserve host diagnostics so the durable report identifies the cause.
pub(super) fn gate_assessment(result: &HostCommandResult) -> (String, Vec<String>) {
    if result.reusable() {
        return ("accepted".into(), Vec::new());
    }
    if let Some(envelope) = &result.gate_envelope {
        if let Some(error) = &envelope.operational_error {
            if error.kind == "input_invalid" {
                return ("needs_remediation".into(), vec![error.text.clone()]);
            }
            return (
                "operational_failure".into(),
                vec![format!(
                    "operational failure ({}): {}; stderr: {}",
                    error.kind, error.text, result.stderr
                )],
            );
        }
        if !envelope.policy_findings.is_empty() {
            return (
                "needs_remediation".into(),
                envelope
                    .policy_findings
                    .iter()
                    .map(|finding| finding.text.clone())
                    .collect(),
            );
        }
        if let Some(postcondition) = &result.postcondition
            && !postcondition.satisfied
        {
            return (
                "needs_remediation".into(),
                vec![postcondition.summary.clone()],
            );
        }
    }
    let cause = if result.timed_out {
        "timed out".to_string()
    } else if result.interrupted {
        "interrupted".to_string()
    } else {
        match result.exit_code {
            Some(code) => format!("exit code {code}"),
            None => "no exit status".to_string(),
        }
    };
    (
        "operational_failure".into(),
        vec![format!(
            "operational failure: gate produced no policy verdict (missing gate envelope); {cause}; stderr: {}",
            result.stderr
        )],
    )
}

pub(super) fn operational_result(error: impl std::fmt::Display) -> HostCommandResult {
    let stderr = error.to_string();
    HostCommandResult {
        exit_code: None,
        stdout: String::new(),
        stderr_bytes: stderr.len() as u64,
        stderr,
        stdout_bytes: 0,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: None,
        publication_receipt: None,
        observed_only: false,
        subjects: Vec::new(),
        postcondition: None,
    }
}

pub(super) fn input_invalid_result(subject: &str, cause: &str) -> HostCommandResult {
    HostCommandResult {
        exit_code: Some(1),
        stdout: String::new(),
        stderr: cause.to_string(),
        stdout_bytes: 0,
        stderr_bytes: cause.len() as u64,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
            schema_version: 1,
            report: serde_json::Value::Null,
            policy_findings: Vec::new(),
            operational_error: Some(archon_workflow::GateOperationalError {
                kind: "input_invalid".into(),
                text: format!("{subject}: {cause}"),
            }),
        }),
        publication_receipt: None,
        observed_only: true,
        subjects: Vec::new(),
        postcondition: None,
    }
}

/// Preserve typed task-body parse failures that the executor discovers while
/// checking host postconditions. These are defects in the judged subject, just
/// like the staged child's input-invalid finding; every other executor error
/// remains operational.
pub(super) fn executor_error_result(
    error: archon_workflow::WorkflowError,
    gate_id: GateId,
    fallback_subject: &str,
) -> HostCommandResult {
    let error = anyhow::Error::new(error);
    let Some(evaluation) =
        workflow_gate::task_body_input_finding(&error, gate_id, fallback_subject)
    else {
        return operational_result(error);
    };
    let envelope = match evaluation.into_envelope() {
        Ok(envelope) => envelope,
        Err(error) => return operational_result(error),
    };
    let stderr = error.to_string();
    HostCommandResult {
        exit_code: None,
        stdout: String::new(),
        stderr_bytes: stderr.len() as u64,
        stderr,
        stdout_bytes: 0,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: Some(envelope),
        publication_receipt: None,
        observed_only: true,
        subjects: Vec::new(),
        postcondition: None,
    }
}
