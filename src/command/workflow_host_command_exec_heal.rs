use super::*;
use crate::command::{workflow_host_fault as fault, workflow_host_retry};

impl FixedHostCommandExecutor {
    pub(super) async fn execute_inner(
        &self,
        request: HostCommandRequest,
        expected_generation: Option<u64>,
        existing_receipt: Option<archon_workflow::PublicationReceiptV1>,
        judge_only: bool,
    ) -> WorkflowResult<HostCommandResult> {
        self.execute_inner_claimed(
            request,
            expected_generation,
            existing_receipt,
            judge_only,
            None,
        )
        .await
    }

    pub(super) async fn execute_inner_claimed(
        &self,
        request: HostCommandRequest,
        expected_generation: Option<u64>,
        existing_receipt: Option<archon_workflow::PublicationReceiptV1>,
        judge_only: bool,
        claim: Option<crate::command::workflow_host_command_committed::Claim>,
    ) -> WorkflowResult<HostCommandResult> {
        let generation = expected_generation.ok_or_else(|| {
            WorkflowError::StateCorrupt(
                "fixed HostCommand execution has no dispatch generation".into(),
            )
        })?;
        let store = archon_workflow::WorkflowStore::project(&self.context.project_root);
        let run_id = self
            .run_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt("host command run root has no UTF-8 identity".into())
            })?;
        let attempts = std::sync::atomic::AtomicU32::new(0);
        let result = workflow_host_retry::retry(&request.command_id,
            (&store, run_id, generation, self.paused_run_authorization.as_ref()), || async {
                crate::command::workflow_host_command_integrity::require_launch_prd_unchanged(&self.context, &request.command_id)?;
                let attempt = attempts.fetch_update(std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst, |n| Some(n.saturating_add(1))).expect("attempt counter update").saturating_add(1);
                let mut outcome = match self.execute_once(request.clone(), expected_generation, existing_receipt.clone(), judge_only, claim.as_ref(), attempt).await {
                    Err(error @ (WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_) | WorkflowError::TaskBodyInvalid(_) | WorkflowError::StageFailed(_))) => return Err(error.into()),
                    Err(WorkflowError::Io { path, source }) if path == self.context.program && source.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(permanent_outcome(format!("program not found at {}: {source}; remedy: install the executable at {} and retry", path.display(), path.display())));
                    }
                    Err(error) => {
                        if fault::evidence_refused(&error) { return Err(error.into()); }
                        let error = anyhow::Error::new(error);
                        if fault::classify(&error) == fault::Class::Transient { return Err(error); }
                        return Ok(permanent_outcome(fault::diagnostic(&error)));
                    }
                    Ok(outcome) => outcome,
                };
                if outcome.gate_envelope.is_none() && outcome.exit_code != Some(0) {
                    let error = serde_json::from_str::<archon_workflow::GateOperationalError>(&outcome.stdout).ok()
                        .unwrap_or_else(|| archon_workflow::GateOperationalError {
                            kind: fault::Class::Permanent.kind().into(),
                            text: if outcome.stderr.is_empty() { format!("host command {} exited {:?} without a gate envelope", request.command_id, outcome.exit_code) } else { outcome.stderr.clone() },
                        });
                    outcome.gate_envelope = Some(GateEnvelopeV1 { schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION, report: serde_json::Value::Null, policy_findings: Vec::new(), operational_error: Some(error) });
                }
                let Some(envelope) = &mut outcome.gate_envelope else { return Ok(outcome); };
                // Only an entirely transient operational outcome may be withheld.
                if envelope.policy_findings.iter().any(|finding| finding.remediation_scope != archon_workflow::RemediationScope::Operational) { return Ok(outcome); }
                let permanent = envelope.policy_findings.iter().filter(|finding| fault::finding_class(finding) == fault::Class::Permanent).map(|finding| finding.text.as_str()).collect::<Vec<_>>();
                if !permanent.is_empty() {
                    let reasons = permanent.join("; ");
                    let text = envelope.operational_error.as_ref().map_or_else(|| reasons.clone(), |error| format!("{}; {reasons}", error.text));
                    envelope.operational_error = Some(archon_workflow::GateOperationalError { kind: fault::Class::Permanent.kind().into(), text });
                    return Ok(outcome);
                }
                if let Some(error) = &mut envelope.operational_error {
                    if matches!(error.kind.as_str(), "input_invalid" | "authored_judge_reply") { return Ok(outcome); }
                    if matches!(error.kind.as_str(), "transient" | "transient_host_fault") { return Err(fault::transient(error.text.clone())); }
                    // A legacy/unknown kind is permanent; keep its sealed evidence unchanged.
                    return Ok(outcome);
                }
                if !envelope.policy_findings.is_empty() && envelope.policy_findings.iter().all(|finding| fault::finding_class(finding) == fault::Class::Transient) {
                    return Err(fault::transient(envelope.policy_findings.iter().map(|finding| finding.text.as_str()).collect::<Vec<_>>().join("; ")));
                }
                Ok(outcome)
            }).await;
        result.map_err(|error| {
            error
                .downcast::<WorkflowError>()
                .unwrap_or_else(|error| WorkflowError::StageFailed(format!("{error:#}")))
        })
    }
}

fn permanent_outcome(text: String) -> HostCommandResult {
    unpublished(
        None,
        String::new(),
        String::new(),
        (0, 0),
        GateEnvelopeV1 {
            schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
            report: serde_json::Value::Null,
            policy_findings: Vec::new(),
            operational_error: Some(archon_workflow::GateOperationalError {
                kind: fault::Class::Permanent.kind().into(),
                text,
            }),
        },
        "host capability requires repair",
    )
}
