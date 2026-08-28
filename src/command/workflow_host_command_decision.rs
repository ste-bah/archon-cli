//! Whether a staged host command publishes, refuses, or is held back.

use archon_workflow::{
    CommandPostconditionEvaluation, GATE_ENVELOPE_SCHEMA_VERSION, GateEnvelopeV1,
    GatePolicyFinding, HostCommandResult, PreparedPublicationV1, RemediationScope,
};

use super::workflow_host_command_catalog::ResolvedHostCommand;

/// The one declared output every staged command writes, refusal or not.
const GATE_ENVELOPE_OUTPUT: &str = "gate-envelope.json";

/// Whether the command refused the candidate instead of staging a publication.
///
/// A refusal stages the gate envelope alone: there is no contract, lock or pin
/// to publish, only the finding that says why. The audit demands the staged
/// tree equal the whole declared write set, so without this the refusal would
/// surface as an integrity failure and kill the run — which is exactly what a
/// findings-driven retry exists to avoid. A refusal must still carry a finding;
/// staging nothing silently is an integrity failure, not a refusal.
pub(crate) fn candidate_refused_before_staging(
    prepared: &PreparedPublicationV1,
    command: &ResolvedHostCommand,
) -> bool {
    let envelope_only = prepared.entries.len() == 1
        && prepared.entries[0].relative_path == GATE_ENVELOPE_OUTPUT
        && command.declared_write_set.len() > 1;
    envelope_only
}

pub(crate) fn candidate_findings_prevent_publication(
    command_id: &str,
    mode: archon_core::config::GateMode,
    envelope: &GateEnvelopeV1,
) -> bool {
    envelope.policy_findings.iter().any(|finding| {
        if matches!(
            finding.remediation_scope,
            RemediationScope::PrdInput | RemediationScope::Operational
        ) {
            return true;
        }
        if mode == archon_core::config::GateMode::Observe {
            return false;
        }
        match command_id {
            "freeze-acceptance" => finding.remediation_scope == RemediationScope::CandidateArtifact,
            "freeze-skeleton" => matches!(
                finding.remediation_scope,
                RemediationScope::CandidateArtifact | RemediationScope::Skeleton
            ),
            "land-task-body" => finding.remediation_scope != RemediationScope::InheritedPredecessor,
            _ => false,
        }
    })
}

/// A completed host command that deliberately published nothing.
///
/// Refusals, operational failures and finding-blocked candidates all end the
/// same way: the envelope explains why, no receipt exists, and the postcondition
/// is unsatisfied. Building that shape once keeps the three reasons from drifting
/// apart in the record the script and the operator read.
pub(crate) fn unpublished(
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    byte_counts: (u64, u64),
    envelope: GateEnvelopeV1,
    summary: &str,
) -> HostCommandResult {
    HostCommandResult {
        exit_code,
        stdout,
        stderr,
        stdout_bytes: byte_counts.0,
        stderr_bytes: byte_counts.1,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: Some(envelope),
        publication_receipt: None,
        subjects: Vec::new(),
        postcondition: Some(CommandPostconditionEvaluation {
            satisfied: false,
            summary: summary.to_string(),
        }),
    }
}

/// The envelope the host writes when it refuses a candidate before any command
/// ran.
///
/// The gate normally authors findings, but a body that binds to no frozen
/// subject never reaches it: the host is the authority that noticed, so it says
/// so in the same shape, and the author gets its next attempt.
pub(crate) fn candidate_refusal_envelope(command_id: &str, reason: &str) -> GateEnvelopeV1 {
    GateEnvelopeV1 {
        schema_version: GATE_ENVELOPE_SCHEMA_VERSION,
        report: serde_json::json!(format!("{command_id} refused the candidate before staging")),
        policy_findings: vec![GatePolicyFinding {
            text: reason.to_string(),
            subject: command_id.to_string(),
            source_path: None,
            remediation_scope: RemediationScope::CandidateArtifact,
        }],
        operational_error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::PreparedPublicationEntry;
    use std::path::PathBuf;

    fn command(outputs: &[&str]) -> ResolvedHostCommand {
        ResolvedHostCommand {
            command_id: "freeze-acceptance".into(),
            program: PathBuf::from("/trusted/archon"),
            args: Vec::new(),
            cwd: PathBuf::from("/project"),
            environment: Default::default(),
            stdin: None,
            timeout_secs: 1,
            max_stdout_bytes: 1,
            max_stderr_bytes: 1,
            declared_write_set: outputs.iter().map(PathBuf::from).collect(),
            remediation_scopes: Default::default(),
        }
    }

    fn prepared(paths: &[&str]) -> PreparedPublicationV1 {
        PreparedPublicationV1 {
            schema_version: archon_workflow::PREPARED_PUBLICATION_SCHEMA_VERSION,
            call_id: "call".into(),
            command_id: "freeze-acceptance".into(),
            entries: paths
                .iter()
                .map(|path| PreparedPublicationEntry {
                    relative_path: (*path).to_string(),
                    byte_len: 1,
                    blake3: "d".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn staging_only_the_envelope_is_a_refusal_not_a_publication() {
        let declared = command(&[
            "/s/acceptance-contract.json",
            "/s/acceptance-contract.lock",
            "/s/gate-envelope.json",
        ]);

        assert!(candidate_refused_before_staging(
            &prepared(&["gate-envelope.json"]),
            &declared
        ));
    }

    #[test]
    fn a_full_staged_write_set_still_publishes() {
        let declared = command(&[
            "/s/acceptance-contract.json",
            "/s/acceptance-contract.lock",
            "/s/gate-envelope.json",
        ]);

        assert!(!candidate_refused_before_staging(
            &prepared(&[
                "acceptance-contract.json",
                "acceptance-contract.lock",
                "gate-envelope.json",
            ]),
            &declared
        ));
    }

    #[test]
    fn a_command_whose_only_output_is_the_envelope_is_never_a_refusal() {
        let declared = command(&["/s/gate-envelope.json"]);

        assert!(!candidate_refused_before_staging(
            &prepared(&["gate-envelope.json"]),
            &declared
        ));
    }
}
