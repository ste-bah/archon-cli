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

/// The subject a task body landing projects when its outcome names no task:
/// at its start, on an interruption, or when it fails before binding one.
const BODY_PLACEHOLDER: &str = "body";
const LAND_TASK_BODY: &str = "land-task-body";

/// How a host command's projection keeps `dispositions` (Issue-258): whether
/// its own entry is stored, and which entries it supersedes.
///
/// - A landing's subject is its task id, which only its outcome names. The
///   placeholder it projects before that is never stored: its completion,
///   keyed by the task id, could never clear it.
/// - A command with a fixed subject (`acceptance`, `skeleton`, ...) that
///   finished or was interrupted supersedes the entry keyed by its command id
///   that an earlier projection of the same command wrote (an outright
///   failure, or the old projection of an interruption).
/// - Any landing supersedes a stored placeholder. Once it names its task,
///   it also supersedes the legacy command-keyed failure.
pub(super) fn host_bookkeeping(
    command_id: &str,
    subject: &str,
    started: bool,
) -> (bool, Vec<String>) {
    if command_id == LAND_TASK_BODY {
        let mut superseded = vec![BODY_PLACEHOLDER.to_string()];
        if !started && subject != BODY_PLACEHOLDER {
            superseded.push(LAND_TASK_BODY.to_string());
        }
        return (subject != BODY_PLACEHOLDER, superseded);
    }
    let superseded = (!started && subject != command_id)
        .then(|| command_id.to_string())
        .into_iter()
        .collect();
    (true, superseded)
}

/// Reconcile current host records in memory, including successful resumes
/// whose projection has not yet caught up. Real failures without a bound task
/// remain visible; interrupted calls and accepted replacements clear stale
/// legacy entries without writing anything during status.
pub(crate) fn reconcile_interrupted(
    dispositions: &mut std::collections::BTreeMap<String, SubjectDisposition>,
    records: &[WorkflowV2CallRecord],
) -> WorkflowResult<()> {
    let unbound_landing_failed = records.iter().any(|record| {
        record.status == WorkflowV2Status::Failed
            && record
                .call
                .options
                .host_command
                .as_ref()
                .is_some_and(|request| request.command_id == LAND_TASK_BODY)
            && serde_json::from_value::<HostCommandResult>(record.result.data.clone())
                .map_or(true, |outcome| outcome.subjects.is_empty())
    });
    let mut latest = std::collections::BTreeMap::new();
    for record in records {
        let Some(request) = &record.call.options.host_command else {
            continue;
        };
        if request.command_id == LAND_TASK_BODY {
            continue;
        }
        let (_, subject) = host_subject(&request.command_id, &empty_outcome());
        let entry = latest.entry(subject).or_insert(record);
        if supersession_order(record) > supersession_order(entry) {
            *entry = record;
        }
    }
    for (subject, record) in latest {
        // Rebuild the latest call's complete disposition. An interruption is
        // only authoritative until a newer call for this fixed subject exists.
        for old in records {
            if let Some(request) = &old.call.options.host_command
                && host_subject(&request.command_id, &empty_outcome()).1 == subject
            {
                dispositions.remove(&request.command_id);
            }
        }
        dispositions.remove(&subject);
        let kind = if interruption_reason(record).is_some() {
            super::FixedCallProjectionKind::Interrupted
        } else if record.status == WorkflowV2Status::Running {
            super::FixedCallProjectionKind::Started
        } else {
            super::FixedCallProjectionKind::Executed
        };
        let projected = super::projection(record, kind)?;
        if let Some((key, value)) = projected.disposition {
            dispositions.insert(key, value);
        }
        // A later author attempt is work in progress for the same subject.
        // Replaying an older gate must not overwrite that attempt either.
        if let Some(author) = records
            .iter()
            .filter(|current| {
                current.call.method == archon_workflow::WorkflowV2HostMethod::Agent
                    && author_subject(&current.call.id).1 == subject
                    && current.started_at > record.started_at
            })
            .max_by_key(|current| supersession_order(current))
        {
            dispositions.insert(
                subject,
                if interruption_reason(author).is_some() {
                    SubjectDisposition::Interrupted
                } else {
                    SubjectDisposition::Pending
                },
            );
        }
    }
    for record in records {
        let Some(request) = record.call.options.host_command.as_ref() else {
            continue;
        };
        if request.command_id != LAND_TASK_BODY {
            continue;
        }
        if interruption_reason(record).is_none() {
            if archon_workflow::v2::script::is_reusable_status(record.status)
                && let Ok(outcome) =
                    serde_json::from_value::<HostCommandResult>(record.result.data.clone())
            {
                let (_, subject) = host_subject(&request.command_id, &outcome);
                let (_, superseded) = host_bookkeeping(&request.command_id, &subject, false);
                for key in superseded {
                    if key != LAND_TASK_BODY || !unbound_landing_failed {
                        dispositions.remove(&key);
                    }
                }
            }
            continue;
        }
        let (_, subject) = host_subject(&request.command_id, &empty_outcome());
        let (persist, superseded) = host_bookkeeping(&request.command_id, &subject, false);
        for key in superseded {
            dispositions.remove(&key);
        }
        if persist {
            dispositions.insert(subject, SubjectDisposition::Interrupted);
        } else if !unbound_landing_failed {
            dispositions.remove(&request.command_id);
        }
    }
    if unbound_landing_failed {
        dispositions.insert(LAND_TASK_BODY.to_string(), SubjectDisposition::Failed);
    }
    Ok(())
}

/// Call time establishes supersession across record slots. Legacy records may
/// have no timestamps: an ambiguous interruption cannot displace an answer,
/// and an ambiguous successful answer cannot erase a real failure.
fn supersession_order(record: &WorkflowV2CallRecord) -> (&str, &str, u8, &str) {
    let finality = if interruption_reason(record).is_some()
        || matches!(
            record.status,
            WorkflowV2Status::Pending | WorkflowV2Status::Running
        ) {
        0
    } else if matches!(
        record.status,
        WorkflowV2Status::Failed | WorkflowV2Status::Cancelled
    ) {
        2
    } else {
        1
    };
    (
        &record.started_at,
        &record.finished_at,
        finality,
        &record.call.id,
    )
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
            .unwrap_or_else(|| BODY_PLACEHOLDER.to_string()),
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
