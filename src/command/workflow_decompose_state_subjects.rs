//! Subject, phase and disposition helpers for the fixed-decomposition
//! projection, and the operator-log line writer. Split from
//! `workflow_decompose_state.rs` to keep it under the file-size limit.

use std::path::Path;

use archon_workflow::{
    DecompositionPhase, HostCommandResult, SubjectDisposition, WorkflowResult,
    WorkflowV2CallRecord, WorkflowV2Status,
};

/// Why a call stopped without a result, when its record says so: the
/// `interrupted` reason (`paused`, `cancelled`, an orphaned host, ...) an
/// interrupted record carries (Issue-258). Such a record stays `NeedsReview`
/// -- never reusable, so a resume re-runs the call -- but it says nothing
/// about the work, so it is shown as interrupted, never as failed.
pub(crate) fn interruption_reason(record: &WorkflowV2CallRecord) -> Option<&str> {
    if archon_workflow::v2::script::is_reusable_status(record.status) {
        return None;
    }
    record.result.data.get("interrupted")?.as_str()
}

/// A host command outcome with nothing in it: what a call that has not
/// produced one projects its subject from.
pub(super) fn empty_outcome() -> HostCommandResult {
    HostCommandResult {
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        stdout_bytes: 0,
        stderr_bytes: 0,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: None,
        publication_receipt: None,
        subjects: Vec::new(),
        postcondition: None,
    }
}

pub(super) fn author_subject(call_id: &str) -> (DecompositionPhase, String) {
    if call_id.starts_with("acceptance-author-") {
        (DecompositionPhase::Acceptance, "acceptance".to_string())
    } else if call_id.starts_with("skeleton-author-") {
        (DecompositionPhase::Skeleton, "skeleton".to_string())
    } else {
        let subject = call_id
            .strip_prefix("body-")
            .and_then(|value| value.rsplit_once("-author-").map(|(subject, _)| subject))
            .unwrap_or(call_id)
            .to_string();
        (DecompositionPhase::Bodies, subject)
    }
}

pub(super) fn command_phase(command_id: &str) -> DecompositionPhase {
    match command_id {
        "freeze-acceptance" | "verify-frozen-acceptance" => DecompositionPhase::Acceptance,
        "freeze-skeleton" | "verify-frozen-skeleton" => DecompositionPhase::Skeleton,
        "land-task-body" => DecompositionPhase::Bodies,
        "task-set-lint" | "requirements-trace" => DecompositionPhase::SetGates,
        _ => DecompositionPhase::Reconciliation,
    }
}

pub(super) fn host_subject(
    command_id: &str,
    outcome: &HostCommandResult,
) -> (DecompositionPhase, String) {
    let subject = match command_id {
        "freeze-acceptance" | "verify-frozen-acceptance" => "acceptance".to_string(),
        "freeze-skeleton" | "verify-frozen-skeleton" => "skeleton".to_string(),
        "land-task-body" => outcome
            .subjects
            .first()
            .map(|subject| subject.task_id.clone())
            .unwrap_or_else(|| "body".to_string()),
        other => other.to_string(),
    };
    (command_phase(command_id), subject)
}

pub(super) fn trailing_attempt(call_id: &str) -> Option<u32> {
    call_id.rsplit('-').next()?.parse().ok()
}

pub(super) fn disposition_from_status(
    status: WorkflowV2Status,
    findings: bool,
    committed: bool,
) -> SubjectDisposition {
    match status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop if findings && committed => {
            SubjectDisposition::AcceptedWithShadowFindings
        }
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop if committed => {
            SubjectDisposition::Accepted
        }
        WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => SubjectDisposition::Failed,
        WorkflowV2Status::Blocked => SubjectDisposition::Blocked,
        WorkflowV2Status::NeedsReview if findings && committed => {
            SubjectDisposition::AcceptedWithShadowFindings
        }
        _ => SubjectDisposition::Pending,
    }
}

pub(super) fn append_log(
    path: &Path,
    seq: u64,
    detail: &serde_json::Value,
    finding_texts: &[String],
) -> WorkflowResult<()> {
    let phase = detail["phase"].as_str().unwrap_or("unknown");
    let subject = detail["subject"].as_str().unwrap_or("none");
    let (subject_key, subject_value) = if phase == "bodies" {
        (
            "subject_digest",
            archon_workflow::task_set_contract::content_digest(subject.as_bytes()),
        )
    } else {
        ("subject", subject.to_string())
    };
    let line = format!(
        "event_id={seq} phase={phase} {subject_key}={subject_value} attempt={} disposition={} findings={} status={} reused={}\n",
        detail["logical_attempt"]
            .as_u64()
            .map_or_else(|| "none".to_string(), |v| v.to_string()),
        detail["disposition"].as_str().unwrap_or("none"),
        detail["finding_count"].as_u64().unwrap_or(0),
        detail["status"].as_str().unwrap_or("unknown"),
        detail["reused"].as_bool().unwrap_or(false),
    );
    crate::command::workflow_decompose_log::append_nofollow_line(path, line.trim_end())?;
    // One line per finding, carrying its exact text. The summary line above
    // reports how many; without these an operator reading the durable log of a
    // defective task set sees `findings=3` and has nothing to act on.
    for (index, text) in finding_texts.iter().enumerate() {
        let finding_line = format!(
            "event_id={seq} phase={phase} {subject_key}={subject_value} finding={}/{} text={}",
            index + 1,
            finding_texts.len(),
            crate::command::workflow_decompose_events::log_field(text)
        );
        crate::command::workflow_decompose_log::append_nofollow_line(path, &finding_line)?;
    }
    Ok(())
}
