//! Decomposition events that are not a call's own projection.
//!
//! `project_fixed_call` maps one call record to exactly one event, so
//! transitions that are not themselves calls -- a phase boundary, a provider
//! request going in flight, findings being observed -- had no way to reach
//! `events.jsonl` or `.decompose.log` at all. Three event kinds were declared
//! and never constructed as a result.

use std::path::Path;

use archon_workflow::{
    WorkflowEventKind, WorkflowEventLog, WorkflowResult, WorkflowStore,
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
    let line = format!("event_id={seq} phase={phase} subject={subject} event={label}");
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
    let redacted = match archon_workflow::events::sanitize_value(serde_json::Value::String(
        value.to_string(),
    )) {
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

