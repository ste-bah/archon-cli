//! Decomposition events that are not a call's own projection.
//!
//! `project_fixed_call` maps one call record to exactly one event, so
//! transitions that are not themselves calls -- a phase boundary, a provider
//! request going in flight, findings being observed -- had no way to reach
//! `events.jsonl` or `.decompose.log` at all. Three event kinds were declared
//! and never constructed as a result.

use std::path::Path;

use archon_workflow::{
    DecompositionPhase, SubjectDisposition, WorkflowEventKind, WorkflowEventLog, WorkflowResult,
    WorkflowStore,
};

/// Emits one durable event plus its operator-log line.
///
/// Ordered before or after the call's own event by the caller, so the log reads
/// in the order the transitions happened.
pub(crate) fn emit_auxiliary(
    store: &WorkflowStore,
    run_id: &str,
    log_path: &Path,
    kind: WorkflowEventKind,
    label: &str,
    phase: &str,
    subject: &str,
) -> WorkflowResult<()> {
    let detail = archon_workflow::events::sanitize_value(serde_json::json!({
        "phase": phase,
        "subject": subject,
        "event": label,
    }));
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(run_id, seq, kind, detail)?;
    // `transition`, not `event`: the log's `event` field discriminates run
    // markers, and reusing it made every transition line parse as a malformed
    // marker.
    let line = format!("event_id={seq} phase={phase} subject={subject} transition={label}");
    crate::command::workflow_decompose_log::append_nofollow_line(log_path, &line)
}

/// Records that a resume is proceeding on a different build than the one that
/// launched the run (Issue-59). The persisted identity stays the launch
/// record; this event and its log line are the record of the drift. Written
/// before the lifecycle transition, so a failure to record leaves the run
/// paused and untouched.
pub(crate) fn emit_binary_revision_drift(
    store: &WorkflowStore,
    run_id: &str,
    log_path: &Path,
    drift: &archon_workflow::BinaryRevisionDrift,
) -> WorkflowResult<()> {
    let detail = archon_workflow::events::sanitize_value(serde_json::json!({
        "event": "binary_revision_drift",
        "persisted": drift.persisted,
        "current": drift.current,
    }));
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(
        run_id,
        seq,
        WorkflowEventKind::BinaryRevisionDrift,
        detail,
    )?;
    let line = format!(
        "event_id={seq} transition=binary_revision_drift persisted={} current={}",
        log_field(&drift.persisted),
        log_field(&drift.current)
    );
    crate::command::workflow_decompose_log::append_nofollow_line(log_path, &line)
}

pub(crate) fn scope_label(scope: archon_workflow::RemediationScope) -> &'static str {
    use archon_workflow::RemediationScope as Scope;
    match scope {
        Scope::CandidateArtifact => "candidate_artifact",
        Scope::Skeleton => "skeleton",
        Scope::PrdInput => "prd_input",
        Scope::Body => "body",
        Scope::InheritedPredecessor => "inherited_predecessor",
        Scope::Operational => "operational",
    }
}

/// Finding text reaches a durable file and is ultimately model-influenced, so
/// it gets the same treatment as any other event payload: secret redaction
/// first, then the control-character and `=` rules the log marker already
/// applies to its own fields.
pub(crate) fn log_field(value: &str) -> String {
    let redacted =
        match archon_workflow::events::sanitize_value(serde_json::Value::String(value.to_string()))
        {
            serde_json::Value::String(text) => text,
            other => other.to_string(),
        };
    redacted
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|ch| !ch.is_control())
        .take(1024)
        .collect()
}

pub(crate) fn phase_label(phase: DecompositionPhase) -> &'static str {
    match phase {
        DecompositionPhase::Identity => "identity",
        DecompositionPhase::Acceptance => "acceptance",
        DecompositionPhase::Skeleton => "skeleton",
        DecompositionPhase::Bodies => "bodies",
        DecompositionPhase::SetGates => "set_gates",
        DecompositionPhase::Reconciliation => "reconciliation",
        DecompositionPhase::Completed => "completed",
    }
}

pub(crate) fn disposition_label(value: SubjectDisposition) -> &'static str {
    match value {
        SubjectDisposition::Pending => "pending",
        SubjectDisposition::Accepted => "accepted",
        SubjectDisposition::AcceptedWithShadowFindings => "accepted_with_shadow_findings",
        SubjectDisposition::Failed => "failed",
        SubjectDisposition::Blocked => "blocked",
        SubjectDisposition::Interrupted => "interrupted",
    }
}
