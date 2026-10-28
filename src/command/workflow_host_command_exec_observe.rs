//! Once committed, only observation is retryable, including after resume.
use super::*;
use crate::command::{workflow_host_command_committed as committed, workflow_host_retry};

impl FixedHostCommandExecutor {
    pub(super) async fn recover_committed(
        &self,
        request: &HostCommandRequest,
        context: &HostCommandResolutionContext,
        call_id: &str,
        generation: u64,
        claim: Option<&committed::Claim>,
    ) -> WorkflowResult<Option<HostCommandResult>> {
        let Some(receipt) =
            committed::load(&self.run_root, context, call_id, &request.command_id, claim)?
        else {
            return Ok(None);
        };
        let command = self.resolved(request, context, call_id)?;
        let secrets = HostSecrets::of(context, &command.environment);
        let mut outcome = unpublished(
            Some(0),
            String::new(),
            String::new(),
            (0, 0),
            empty_envelope(),
            "committed publication observation pending",
        );
        outcome.gate_envelope = None;
        outcome.publication_receipt = Some(receipt);
        self.observe_committed(context, &command, outcome, generation, &secrets)
            .await
            .map(Some)
            .map_err(|error| secrets.error(error))
    }

    pub(super) async fn observe_committed(
        &self,
        context: &HostCommandResolutionContext,
        command: &ResolvedHostCommand,
        mut outcome: HostCommandResult,
        generation: u64,
        secrets: &HostSecrets,
    ) -> WorkflowResult<HostCommandResult> {
        use anyhow::Context;
        let receipt = outcome.publication_receipt.clone().ok_or_else(|| {
            WorkflowError::StateCorrupt("publication observation has no committed receipt".into())
        })?;
        let store = archon_workflow::WorkflowStore::project(&context.project_root);
        let run_id = self
            .run_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt("publication observation has no run identity".into())
            })?;
        let observed = workflow_host_retry::retry(&command.command_id,
            (&store, run_id, generation, self.paused_run_authorization.as_ref()), || std::future::ready((|| -> anyhow::Result<_> {
                crate::command::workflow_host_command_operational::require_run_owned_authorized(
                    &store, run_id, generation, self.paused_run_authorization.as_ref(),
                )?;
                #[cfg(test)]
                crate::command::workflow_host_fault_test_hooks::fire(
                    crate::command::workflow_host_fault_test_hooks::Point::AfterCommit, &context.task_root,
                )?;
                let destinations = self.destinations(context, command, &receipt.call_id)?;
                committed::validate_destinations(&receipt, &destinations)?;
                // A flush failure can leave the committed journal for settlement.
                let _read = crate::command::workflow_task_set::ChainRead::of(&context.project_root, &context.task_root)?;
                let mut gate = None;
                for entry in &receipt.entries {
                    let path = std::path::PathBuf::from(&entry.destination_path);
                    let pin = crate::command::workflow_task_set::acceptance_pin_path(&context.project_root, &context.task_root);
                    let scopes = [(context.task_root.clone(), None), (pin.parent().expect("pin parent").to_path_buf(), None), (self.run_root.clone(), None)];
                    crate::command::workflow_task_set::validate_destination(&path, &scopes)
                        .with_context(|| format!("published artifact {} has invalid authority; restore its authenticated path", path.display()))?;
                    let metadata = std::fs::symlink_metadata(&path).map_err(|source| WorkflowError::Io { path: path.clone(), source })?;
                    if !metadata.is_file() || metadata.file_type().is_symlink() {
                        return Err(WorkflowError::ArtifactInvalid(format!("published artifact {} is not a regular file; restore its receipt-verified bytes", path.display())).into());
                    }
                    let bytes = std::fs::read(&path).map_err(|source| WorkflowError::Io { path: path.clone(), source })?;
                    if bytes.len() as u64 != entry.byte_len || archon_workflow::task_set_contract::content_digest(&bytes) != entry.blake3 {
                        return Err(WorkflowError::StateCorrupt(format!("published artifact {} differs from receipt {}; restore {} from this authenticated publication", path.display(), receipt.call_id, path.display())).into());
                    }
                    if entry.relative_path == ENVELOPE_FILE {
                        owner_only(&path)?;
                        gate = Some(secrets.envelope(serde_json::from_slice::<GateEnvelopeV1>(&bytes)
                            .with_context(|| format!("reading committed envelope {}; restore its receipt-verified bytes", path.display()))?));
                    }
                }
                let gate = gate.ok_or_else(|| WorkflowError::StateCorrupt(format!("committed receipt {} has no gate envelope; restore its authenticated publication metadata", receipt.call_id)))?;
                let (subjects, postcondition) = crate::command::workflow_host_command_postcondition::evaluate_postcondition_locked(context, &command.command_id)?;
                Ok((gate, subjects, postcondition))
            })())).await;
        match observed {
            Ok((gate, subjects, postcondition)) => {
                outcome.gate_envelope = Some(gate);
                outcome.subjects = subjects;
                outcome.postcondition = Some(postcondition);
            }
            Err(error) => {
                if matches!(
                    error.downcast_ref::<WorkflowError>(),
                    Some(WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_))
                ) {
                    return Err(error
                        .downcast::<WorkflowError>()
                        .expect("checked control error"));
                }
                let text = secrets.text(&crate::command::workflow_host_fault::diagnostic(&error));
                let gate = outcome.gate_envelope.get_or_insert_with(empty_envelope);
                gate.operational_error = Some(archon_workflow::GateOperationalError {
                    kind: "permanent_host_fault".into(),
                    text: text.clone(),
                });
                outcome.postcondition = Some(archon_workflow::CommandPostconditionEvaluation {
                    satisfied: false,
                    summary: text,
                });
            }
        }
        Ok(outcome)
    }
}

fn empty_envelope() -> GateEnvelopeV1 {
    GateEnvelopeV1 {
        schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
        report: serde_json::Value::Null,
        policy_findings: Vec::new(),
        operational_error: None,
    }
}
