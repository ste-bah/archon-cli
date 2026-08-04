//! The activity update a running stage owes its user interface.
//!
//! Every field of the emitted [`WorkflowActivityUpdate`] is derived from the
//! [`StageRunRequest`] this crate owns, and it goes out through
//! [`crate::ui_sink_port`], so nothing here needs a terminal or a host. It sits
//! beside [`crate::stage_command_policy`] because the two read the same request
//! for the same reason: the tool mode named in the detail line has to be the
//! mode the stage actually runs under, and sharing the predicate is what keeps
//! them from drifting.
//!
//! `required_activity` is required, not best-effort: a sink that cannot take
//! the update fails the stage with [`WorkflowError::NotificationDelivery`]
//! rather than dropping it, because a run whose progress silently stopped
//! reporting is indistinguishable from a run that stopped.

use std::path::PathBuf;

use crate::stage_command_policy::command_execution_stage;
use crate::{
    SharedWorkflowUiSink, StageKind, StageRunRequest, WorkflowActivityStatus,
    WorkflowActivityUpdate, WorkflowError, WorkflowResult, WorkflowUiEvent,
};

pub async fn required_activity(
    ui_sink: &SharedWorkflowUiSink,
    request: &StageRunRequest,
    agent_name: &str,
    provider_id: &str,
    model: &str,
    status: WorkflowActivityStatus,
    detail: &str,
) -> WorkflowResult<()> {
    ui_sink
        .emit(activity_event(
            request,
            agent_name,
            provider_id,
            model,
            status,
            detail,
        ))
        .await
        .map_err(|error| delivery_error(&error.to_string(), request, status))
}

fn activity_event(
    request: &StageRunRequest,
    agent_name: &str,
    provider_id: &str,
    model: &str,
    status: WorkflowActivityStatus,
    detail: &str,
) -> WorkflowUiEvent {
    WorkflowUiEvent::Activity(WorkflowActivityUpdate {
        id: format!("workflow:{}:{}", request.run_id, request.stage_id),
        name: agent_name.to_string(),
        status,
        detail: Some(activity_detail(request, detail)),
        run_id: Some(request.run_id.clone()),
        provider: Some(provider_id.to_string()),
        model: Some(model.to_string()),
    })
}

fn delivery_error(
    error: &str,
    request: &StageRunRequest,
    status: WorkflowActivityStatus,
) -> WorkflowError {
    WorkflowError::NotificationDelivery(format!(
        "workflow agent activity delivery failed: run_id={} stage_id={} status={status:?}: {error}",
        request.run_id, request.stage_id
    ))
}

pub fn activity_detail(request: &StageRunRequest, detail: &str) -> String {
    let cwd = request_target_repository_root(request)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "default".to_string());
    format!(
        "{detail} stage={} provider_tier={:?} cwd={} tool_mode={}",
        request.stage_id,
        request.provider_tier,
        cwd,
        workflow_tool_mode(request)
    )
}

fn workflow_tool_mode(request: &StageRunRequest) -> &'static str {
    if matches!(request.stage_kind, StageKind::Implementation) || command_execution_stage(request) {
        "full"
    } else {
        "read_only"
    }
}

/// The repository a stage runs against, when the request declares one.
///
/// It travelled here with `activity_detail`, its only caller inside this crate.
/// The binary's dispatch and its tests reach it through this path.
pub fn request_target_repository_root(request: &StageRunRequest) -> Option<PathBuf> {
    request
        .input
        .get("target_repository_root")
        .and_then(|value| value.as_str())
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}
