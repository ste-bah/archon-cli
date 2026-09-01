//! Durable projection of ordinary v3 call records into fixed-decomposition state.
//!
//! The v3 result store remains the execution truth. This projection gives the
//! fixed workflow its phase/attempt/disposition status and append-only operator
//! log without adding a second scheduler or trusting file existence alone.

use std::path::Path;

use archon_workflow::{
    DecompositionAttemptStateV1, DecompositionPhase, FixedDecompositionStateV1, HostCommandResult,
    SubjectDisposition, WorkflowActivityStatus, WorkflowActivityUpdate, WorkflowError,
    WorkflowEventKind, WorkflowEventLog, WorkflowResult, WorkflowStore, WorkflowUiEvent,
    WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2Status,
};

pub(crate) const FIXED_STATE_PATH: &str = "decomposition/state.json";
const MAX_PROGRESS_EVENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixedCallProjectionKind {
    Started,
    Executed,
    Reused,
    Interrupted,
}

pub(crate) fn project_fixed_call(
    store: &WorkflowStore,
    run_id: &str,
    record: &WorkflowV2CallRecord,
    kind: FixedCallProjectionKind,
) -> WorkflowResult<Option<WorkflowUiEvent>> {
    let path = store.run_dir(run_id).join(FIXED_STATE_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    let mut state: FixedDecompositionStateV1 = serde_json::from_slice(&raw)?;
    let projection = projection(record, kind)?;
    let previous_phase = state.phase;
    state.phase = projection.phase;
    if let Some((subject, attempt)) = &projection.attempt {
        state.attempts.insert(subject.clone(), attempt.clone());
    }
    if let Some((subject, disposition)) = &projection.disposition {
        state.dispositions.insert(subject.clone(), *disposition);
    }
    store.write_run_json(run_id, FIXED_STATE_PATH, &state)?;

    let log_path = crate::command::workflow_decompose_log::validated_fixed_log_path(
        Path::new(&state.log_path),
        &state.identity,
    )?;
    let phase_text = crate::command::workflow_decompose_events::phase_label(projection.phase);
    // Body subjects are digested here exactly as `append_log` digests them: the
    // operator log must never carry a raw body subject.
    let raw_subject = projection
        .disposition
        .as_ref()
        .map_or("none", |(subject, _)| subject.as_str());
    let subject_text = if projection.phase == DecompositionPhase::Bodies {
        archon_workflow::task_set_contract::content_digest(raw_subject.as_bytes())
    } else {
        raw_subject.to_string()
    };
    use crate::command::workflow_decompose_events::emit_auxiliary;
    // A phase banner. Nothing marked where one phase ended and the next began,
    // so the log opened straight onto an author attempt with no boundary.
    if projection.phase != previous_phase {
        emit_auxiliary(
            store,
            run_id,
            &log_path,
            WorkflowEventKind::DecompositionPhaseStarted,
            "decomposition_phase_started",
            phase_text,
            &subject_text,
        )?;
    }
    // The provider request itself, distinct from the logical attempt beginning.
    if projection.event_kind == WorkflowEventKind::AuthorAttemptStarted {
        emit_auxiliary(
            store,
            run_id,
            &log_path,
            WorkflowEventKind::ModelCallInFlight,
            "model_call_in_flight",
            phase_text,
            &subject_text,
        )?;
    }

    let detail = serde_json::json!({
        "event": projection.event_label,
        "call_id": record.call.id,
        "method": record.call.method.as_str(),
        "phase": crate::command::workflow_decompose_events::phase_label(projection.phase),
        "subject": projection.disposition.as_ref().map(|(subject, _)| subject),
        "logical_attempt": projection.attempt.as_ref().map(|(_, attempt)| attempt.logical_attempt),
        "disposition": projection.disposition.as_ref().map(|(_, value)| crate::command::workflow_decompose_events::disposition_label(*value)),
        "finding_count": projection.finding_count,
        "status": record.status,
        "reused": projection.reused,
    });
    let sanitized = archon_workflow::events::sanitize_value(detail);
    let encoded = serde_json::to_vec(&sanitized)?;
    if encoded.len() > MAX_PROGRESS_EVENT_BYTES {
        return Err(WorkflowError::StageFailed(format!(
            "fixed decomposition progress event exceeds {MAX_PROGRESS_EVENT_BYTES} bytes"
        )));
    }
    // Decided before the kind is consumed by the emit below.
    let emit_completion = record.call.method == WorkflowV2HostMethod::HostCommand
        && !matches!(kind, FixedCallProjectionKind::Started)
        && projection.event_kind != WorkflowEventKind::HostCommandCompleted;
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(
        run_id,
        seq,
        projection.event_kind,
        sanitized.clone(),
    )?;
    append_log(&log_path, seq, &sanitized, &projection.finding_texts)?;
    // A host command completing and what that completion meant are separate
    // events in the vocabulary. Folding them together left a healthy run with
    // no completion event at all: committed calls became subject_accepted and
    // refused ones author_attempt_rejected.
    if emit_completion {
        emit_auxiliary(
            store,
            run_id,
            &log_path,
            WorkflowEventKind::HostCommandCompleted,
            "host_command_completed",
            phase_text,
            &subject_text,
        )?;
    }
    // Findings observed, whatever the disposition. Tying this to a disposition
    // made it unreachable: every committed call carrying findings is already
    // labelled accepted-with-shadow-findings.
    if projection.finding_count > 0 {
        emit_auxiliary(
            store,
            run_id,
            &log_path,
            WorkflowEventKind::ShadowFindingsObserved,
            "shadow_findings_observed",
            phase_text,
            &subject_text,
        )?;
    }
    Ok(Some(WorkflowUiEvent::Activity(WorkflowActivityUpdate {
        id: format!("decomposition:{run_id}:{}", record.call.id),
        name: format!("fixed decomposition {}", crate::command::workflow_decompose_events::phase_label(projection.phase)),
        status: match record.status {
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop => WorkflowActivityStatus::Complete,
            WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => {
                WorkflowActivityStatus::Failed
            }
            _ => WorkflowActivityStatus::Running,
        },
        detail: Some(format!(
            "{} subject={} attempt={} disposition={} findings={} reused={}",
            projection.event_label,
            sanitized["subject"].as_str().unwrap_or("none"),
            sanitized["logical_attempt"]
                .as_u64()
                .map_or_else(|| "none".to_string(), |value| value.to_string()),
            sanitized["disposition"].as_str().unwrap_or("none"),
            projection.finding_count,
            projection.reused,
        )),
        run_id: Some(run_id.to_string()),
        provider: None,
        model: None,
    })))
}

struct Projection {
    phase: DecompositionPhase,
    attempt: Option<(String, DecompositionAttemptStateV1)>,
    disposition: Option<(String, SubjectDisposition)>,
    event_kind: WorkflowEventKind,
    event_label: &'static str,
    finding_count: usize,
    /// Exact policy-finding text. The count alone told an operator that a phase
    /// had findings but never which, so the durable record of a defective task
    /// set was a number nobody could act on.
    finding_texts: Vec<String>,
    reused: bool,
}

fn projection(
    record: &WorkflowV2CallRecord,
    kind: FixedCallProjectionKind,
) -> WorkflowResult<Projection> {
    let started = kind == FixedCallProjectionKind::Started;
    let interrupted = kind == FixedCallProjectionKind::Interrupted;
    let reused = kind == FixedCallProjectionKind::Reused;
    let call_id = record.call.id.as_str();
    if record.call.method == WorkflowV2HostMethod::Agent {
        let (phase, subject) = author_subject(call_id);
        let logical_attempt = trailing_attempt(call_id).unwrap_or(record.attempt);
        return Ok(Projection {
            phase,
            attempt: Some((
                subject.clone(),
                DecompositionAttemptStateV1 {
                    logical_attempt,
                    interrupted,
                    last_error: (!started && !matches!(record.status, WorkflowV2Status::Accepted))
                        .then(|| record.result.summary.clone()),
                },
            )),
            disposition: if started {
                Some((subject, SubjectDisposition::Pending))
            } else {
                interrupted.then_some((subject, SubjectDisposition::Interrupted))
            },
            event_kind: if started {
                WorkflowEventKind::AuthorAttemptStarted
            } else if interrupted {
                WorkflowEventKind::AuthorAttemptInterrupted
            } else {
                WorkflowEventKind::AuthorAttemptCompleted
            },
            event_label: if started {
                "author_attempt_started"
            } else if interrupted {
                "author_attempt_interrupted"
            } else {
                "author_attempt_completed"
            },
            finding_count: 0,
            finding_texts: Vec::new(),
            reused,
        });
    }
    if record.call.method == WorkflowV2HostMethod::FinalReport {
        return Ok(Projection {
            phase: DecompositionPhase::Completed,
            attempt: None,
            disposition: Some((
                "decomposition".to_string(),
                disposition_from_status(record.status, false, false),
            )),
            event_kind: WorkflowEventKind::DecompositionCompleted,
            event_label: "decomposition_completed",
            finding_count: 0,
            finding_texts: Vec::new(),
            reused,
        });
    }
    if record.call.method != WorkflowV2HostMethod::HostCommand {
        return Ok(Projection {
            phase: DecompositionPhase::Identity,
            attempt: None,
            disposition: None,
            event_kind: WorkflowEventKind::DecompositionPhaseCompleted,
            event_label: "fixed_call_completed",
            finding_count: 0,
            finding_texts: Vec::new(),
            reused,
        });
    }

    let request = record.call.options.host_command.as_ref().ok_or_else(|| {
        WorkflowError::StateCorrupt("fixed HostCommand record has no typed request".to_string())
    })?;
    if started {
        let (phase, subject) = host_subject(
            &request.command_id,
            &HostCommandResult {
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
            },
        );
        return Ok(Projection {
            phase,
            attempt: None,
            disposition: Some((subject, SubjectDisposition::Pending)),
            event_kind: WorkflowEventKind::HostCommandStarted,
            event_label: "host_command_started",
            finding_count: 0,
            finding_texts: Vec::new(),
            reused: false,
        });
    }
    // A host command that failed outright carries an error, not an outcome. The
    // phase still has to be recorded — a failure the operator cannot see in the
    // log is worse than one they can — so an unreadable payload projects as a
    // failed subject rather than aborting the projection and burying the real
    // reason under a deserialisation complaint.
    let Ok(outcome) = serde_json::from_value::<HostCommandResult>(record.result.data.clone())
    else {
        return Ok(Projection {
            phase: command_phase(&request.command_id),
            attempt: None,
            disposition: Some((request.command_id.clone(), SubjectDisposition::Failed)),
            event_kind: WorkflowEventKind::HostCommandCompleted,
            event_label: "host_command_completed",
            finding_count: 0,
            finding_texts: Vec::new(),
            reused: false,
        });
    };
    let (phase, subject) = host_subject(&request.command_id, &outcome);
    let finding_texts = outcome.gate_envelope.as_ref().map_or_else(Vec::new, |envelope| {
        envelope
            .policy_findings
            .iter()
            .map(|finding| {
                format!(
                    "{} [{}] {}",
                    finding.subject,
                    crate::command::workflow_decompose_events::scope_label(finding.remediation_scope),
                    finding.text
                )
            })
            .collect()
    });
    let finding_count = finding_texts.len();
    let committed = outcome.publication_receipt.is_some()
        && outcome
            .postcondition
            .as_ref()
            .is_some_and(|postcondition| postcondition.satisfied);
    let disposition = disposition_from_status(record.status, finding_count > 0, committed);
    // A refusal is a call that ran to completion and did not land its candidate.
    // A cancellation, a crash or a set gate is none of those, and labelling them
    // "the author was rejected" tells an operator the opposite of what happened.
    let candidate_refused = !committed
        && !matches!(
            record.status,
            WorkflowV2Status::Failed
                | WorkflowV2Status::Cancelled
                | WorkflowV2Status::Blocked
        )
        && !crate::command::workflow_host_command_catalog::is_set_gate_command(
            &request.command_id,
        );
    Ok(Projection {
        phase,
        finding_texts,
        attempt: None,
        disposition: Some((subject, disposition)),
        event_kind: if disposition == SubjectDisposition::AcceptedWithShadowFindings {
            WorkflowEventKind::SubjectAcceptedWithShadowFindings
        } else if disposition == SubjectDisposition::Accepted {
            WorkflowEventKind::SubjectAccepted
        } else if candidate_refused {
            // The gate refused this candidate: it never landed and the author
            // gets another attempt. Nothing recorded a rejection, so a phase
            // that burned five attempts looked exactly like one that passed on
            // its first.
            WorkflowEventKind::AuthorAttemptRejected
        } else {
            WorkflowEventKind::HostCommandCompleted
        },
        event_label: if disposition == SubjectDisposition::AcceptedWithShadowFindings {
            "subject_accepted_with_shadow_findings"
        } else if disposition == SubjectDisposition::Accepted {
            "subject_accepted"
        } else if candidate_refused {
            "author_attempt_rejected"
        } else {
            "host_command_completed"
        },
        finding_count,
        reused: false,
    })
}

fn author_subject(call_id: &str) -> (DecompositionPhase, String) {
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

fn command_phase(command_id: &str) -> DecompositionPhase {
    match command_id {
        "freeze-acceptance" => DecompositionPhase::Acceptance,
        "freeze-skeleton" => DecompositionPhase::Skeleton,
        "land-task-body" => DecompositionPhase::Bodies,
        "task-set-lint" | "requirements-trace" => DecompositionPhase::SetGates,
        _ => DecompositionPhase::Reconciliation,
    }
}

fn host_subject(command_id: &str, outcome: &HostCommandResult) -> (DecompositionPhase, String) {
    let subject = match command_id {
        "freeze-acceptance" => "acceptance".to_string(),
        "freeze-skeleton" => "skeleton".to_string(),
        "land-task-body" => outcome
            .subjects
            .first()
            .map(|subject| subject.task_id.clone())
            .unwrap_or_else(|| "body".to_string()),
        other => other.to_string(),
    };
    (command_phase(command_id), subject)
}

fn trailing_attempt(call_id: &str) -> Option<u32> {
    call_id.rsplit('-').next()?.parse().ok()
}

fn disposition_from_status(
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

fn append_log(
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
