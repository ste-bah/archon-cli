//! The fixed executor's live test for a recorded host command, in two
//! strengths: reuse (accepted, no findings) and liveness (findings allowed:
//! the record's outcome is still exactly what is on disk). A resumed run asks
//! the second question about the last landing of a subject, so a freeze the
//! judge gave findings to is not re-judged while its artifact is untouched.
use super::*;

/// The command ran to completion and published; findings are not held
/// against it here.
fn landed(outcome: &HostCommandResult) -> bool {
    outcome.exit_code == Some(0)
        && outcome.publication_receipt.is_some()
        && outcome
            .gate_envelope
            .as_ref()
            .is_some_and(|envelope| envelope.operational_error.is_none())
        && outcome
            .postcondition
            .as_ref()
            .is_some_and(|postcondition| postcondition.satisfied)
        && !outcome.timed_out
        && !outcome.interrupted
        && !outcome.stdout_truncated
        && !outcome.stderr_truncated
}

impl FixedHostCommandExecutor {
    pub(super) fn record_is_reusable_live(
        &self,
        record: &WorkflowV2CallRecord,
        findings_allowed: bool,
    ) -> WorkflowResult<bool> {
        let status_ok = if findings_allowed {
            matches!(
                record.status,
                archon_workflow::WorkflowV2Status::Accepted
                    | archon_workflow::WorkflowV2Status::Noop
                    | archon_workflow::WorkflowV2Status::NeedsReview
            )
        } else {
            matches!(
                record.status,
                archon_workflow::WorkflowV2Status::Accepted
                    | archon_workflow::WorkflowV2Status::Noop
            )
        };
        if record.call.method != archon_workflow::WorkflowV2HostMethod::HostCommand
            || !status_ok
            || record.invalidated_by.is_some()
        {
            return Ok(false);
        }
        let request = record.call.options.host_command.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("persisted HostCommand record has no typed request".into())
        })?;
        if self.call_identity(request)? != record.call.id {
            return Ok(false);
        }
        let outcome: HostCommandResult = serde_json::from_value(record.result.data.clone())?;
        let outcome_ok = if findings_allowed {
            landed(&outcome)
        } else {
            outcome.reusable()
        };
        if !outcome_ok || !receipt_matches_live(outcome.publication_receipt.as_ref())? {
            return Ok(false);
        }
        let context = match self.context_for_request(request) {
            Ok(context) => context,
            // Nothing the host could not bind is reusable.
            Err(WorkflowError::SpecInvalid(_)) => return Ok(false),
            Err(error) => return Err(error),
        };
        let (_, current_postcondition) = evaluate_postcondition(&context, &request.command_id)?;
        if !current_postcondition.satisfied {
            return Ok(false);
        }
        if findings_allowed {
            // Terminality comes from the record itself: a landed outcome with a
            // satisfied postcondition is terminal for this subject. The state
            // file's disposition is not consulted, because a resume replays
            // every record in script order and a refused attempt replayed just
            // before this one leaves it reading `Pending`.
            return Ok(true);
        }
        fixed_subject_is_terminal(&self.run_root, &request.command_id, &outcome)
    }
}
