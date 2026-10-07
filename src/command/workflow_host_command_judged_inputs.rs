//! Issue 337: the content an unpublished host-command outcome judged, so a
//! covered replay gives that outcome again only while the content is the same.
//!
//! A crash pause covers the unpublished refusals of the run, so a resume
//! replays them instead of asking the gate again. A gate that takes a
//! candidate on stdin (a freeze, a body landing) also reads the task root:
//! the frozen chain and the task files beside the candidate. Its call identity
//! binds the candidate, the PRD digest and the paths, NOT that content. A
//! refusal that the task root caused then replayed after the operator repaired
//! the task root, and the script crashed again on the same stale answer.
//!
//! The root cause is a replay that does not know what the answer was about.
//! So the host stamps on the outcome the digest of the content the call judged
//! ([`JUDGED_INPUTS`]), read before and after the call ran, and a covered
//! replay requires the digest now to be the same. The call identity does not
//! change (a changed identity re-keys every recorded call of a live run).
//! Running the gate again instead was not chosen: these gates ask a provider
//! (the freeze and body judges), and a replay exists so that a judge cannot
//! replace its answer about unchanged inputs.
//!
//! No stamp means the content judged is not known, and the outcome never
//! replays: the call runs again. That is the case for an older binary's
//! record, for a read that changed while the call ran, for a candidate the
//! host could not bind, and for a pure host read (a frozen-chain verify), which
//! is cheap to run again and asks no judge.

use archon_workflow::task_set_contract::content_digest;
use archon_workflow::{HostCommandRequest, WorkflowError, WorkflowResult, WorkflowV2CallRecord};

use super::workflow_host_command_catalog::{HostCommandResolutionContext, is_set_gate_command};
use super::workflow_host_command_exec::WorkflowHostCommandExecutor;
use super::workflow_host_command_manifest::set_gate_input_manifest_digest;

/// Where a host command's outcome records the digest of what it judged.
pub(crate) const JUDGED_INPUTS: &str = "judgedInputsDigest";

/// The digest of the content a call of `request` judges in `context`: the
/// task set's gate inputs (the frozen chain, every task file, the PRD digest)
/// and the bytes of the frozen task file it names. `None` for a command that
/// takes no candidate and is no set gate: a pure host read.
pub(crate) fn judged_inputs_digest(
    context: &HostCommandResolutionContext,
    request: &HostCommandRequest,
) -> WorkflowResult<Option<String>> {
    if request.stdin.is_none() && !is_set_gate_command(&request.command_id) {
        return Ok(None);
    }
    let mut canonical = b"judged-inputs-v1\0".to_vec();
    canonical.extend_from_slice(set_gate_input_manifest_digest(context)?.as_bytes());
    canonical.push(0);
    if let Some(task_file) = &context.frozen_task_file {
        let digest = match std::fs::read(task_file) {
            Ok(bytes) => content_digest(&bytes),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => "absent".into(),
            Err(source) => {
                return Err(WorkflowError::Io {
                    path: task_file.clone(),
                    source,
                });
            }
        };
        canonical.extend_from_slice(digest.as_bytes());
    }
    Ok(Some(content_digest(&canonical)))
}

/// What `executor` says a call of `request` judges now. A read that fails is
/// logged and counts as unknown: the outcome is not stamped, or not replayed.
pub(crate) fn read(
    executor: &dyn WorkflowHostCommandExecutor,
    request: &HostCommandRequest,
) -> Option<String> {
    executor.judged_inputs(request).unwrap_or_else(|error| {
        tracing::warn!(%error, command_id = %request.command_id, "judged host-command inputs unreadable; the outcome never replays");
        None
    })
}

/// Stamps `data` with the content judged, only when the reads before and
/// after the call agree: content that changed while the call ran is unknown.
pub(crate) fn stamp(data: &mut serde_json::Value, before: Option<String>, after: Option<String>) {
    if let (Some(before), Some(object)) = (before, data.as_object_mut())
        && after.as_ref() == Some(&before)
    {
        object.insert(JUDGED_INPUTS.into(), before.into());
    }
}

/// Whether the UNPUBLISHED outcome `record` still answers its call: the call
/// identity (candidate, PRD digest, paths) is unchanged, and so is the
/// content it judged.
pub(crate) fn answers_current_inputs(
    executor: &dyn WorkflowHostCommandExecutor,
    record: &WorkflowV2CallRecord,
) -> WorkflowResult<bool> {
    let Some(request) = record.call.options.host_command.as_ref() else {
        return Ok(false);
    };
    let Some(judged) = record.result.data[JUDGED_INPUTS].as_str() else {
        return Ok(false);
    };
    if !super::workflow_host_command_occurrence::record_identity_matches(
        record,
        &executor.call_identity(request)?,
    ) {
        return Ok(false);
    }
    Ok(read(executor, request).as_deref() == Some(judged))
}
