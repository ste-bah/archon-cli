use archon_workflow::{
    SharedWorkflowUiSink, StageKind, StageRunRequest, WorkflowActivityStatus,
    WorkflowActivityUpdate, WorkflowError, WorkflowResult, WorkflowUiEvent,
};

use super::workflow_live_runner::request_target_repository_root;
use archon_workflow::stage_command_policy::command_execution_stage;

pub(super) async fn required_activity(
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

pub(super) fn activity_detail(request: &StageRunRequest, detail: &str) -> String {
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
