use super::*;
use archon_workflow::{WorkflowEventKind, WorkflowEventLog};

pub(in crate::command::workflow_live::workflow_live_v2) fn emit_terminal_event(
    store: &WorkflowStore,
    run_id: &str,
    summary: &WorkflowV2ScriptSummary,
) -> WorkflowResult<()> {
    let kind = terminal_event_kind(summary.status);
    let detail = serde_json::json!({
        "event": "terminal_status",
        "status": summary.status,
        "call_id": summary.failed_call,
        "result_path": summary.failed_result_path,
        "next_action": summary.next_action,
    });
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone())
        .emit(run_id, seq, kind, detail)
        .map(|_| ())
}

pub(in crate::command::workflow_live::workflow_live_v2) fn emit_observer_event(
    store: &WorkflowStore,
    run_id: &str,
    kind: WorkflowEventKind,
    event: &str,
    detail: serde_json::Value,
) -> WorkflowResult<()> {
    if event_label_exists(store, run_id, event)? {
        return Ok(());
    }
    let mut detail = detail.as_object().cloned().unwrap_or_default();
    detail.insert("event".to_string(), serde_json::json!(event));
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone())
        .emit(run_id, seq, kind, serde_json::Value::Object(detail))
        .map(|_| ())
}

pub(in crate::command::workflow_live::workflow_live_v2) fn emit_run_status_event(
    store: &WorkflowStore,
    run_id: &str,
    status: &RunStatus,
    detail: &str,
) -> WorkflowResult<()> {
    let kind = match status {
        RunStatus::Paused => WorkflowEventKind::Paused,
        RunStatus::Cancelled => WorkflowEventKind::Cancelled,
        RunStatus::Failed => WorkflowEventKind::StageFailed,
        RunStatus::Blocked | RunStatus::NeedsReview => WorkflowEventKind::StageStalled,
        RunStatus::Completed => WorkflowEventKind::Completed,
        RunStatus::Planned | RunStatus::Running => WorkflowEventKind::StageStarted,
    };
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone())
        .emit(
            run_id,
            seq,
            kind,
            serde_json::json!({
                "event": "terminal_status",
                "status": status,
                "detail": detail,
            }),
        )
        .map(|_| ())
}

pub(in crate::command::workflow_live::workflow_live_v2) fn event_label_exists(
    store: &WorkflowStore,
    run_id: &str,
    label: &str,
) -> WorkflowResult<bool> {
    let path = store.events_path(run_id);
    let raw = std::fs::read(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    for line in raw
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
    {
        let event = match serde_json::from_slice::<archon_workflow::WorkflowEvent>(line) {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error,
                    "Skipping malformed workflow event during label lookup");
                continue;
            }
        };
        if event
            .detail
            .get("event")
            .and_then(serde_json::Value::as_str)
            == Some(label)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn terminal_event_kind(status: WorkflowV2Status) -> WorkflowEventKind {
    match status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => WorkflowEventKind::StageCompleted,
        WorkflowV2Status::Failed | WorkflowV2Status::Cancelled => WorkflowEventKind::StageFailed,
        WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview => {
            WorkflowEventKind::StageStalled
        }
        WorkflowV2Status::Pending | WorkflowV2Status::Running => WorkflowEventKind::StageStarted,
    }
}
