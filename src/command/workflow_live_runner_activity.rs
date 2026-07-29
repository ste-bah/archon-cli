use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::events::{AgentActivityRole, AgentActivityStatus, AgentActivityUpdate};
use archon_workflow::{StageKind, StageRunRequest, WorkflowError, WorkflowResult};

use super::workflow_live_runner::{command_execution_stage, request_target_repository_root};

pub(super) async fn required_activity(
    tui_tx: &TuiEventSender,
    request: &StageRunRequest,
    agent_name: &str,
    provider_id: &str,
    model: &str,
    status: AgentActivityStatus,
    detail: &str,
) -> WorkflowResult<()> {
    tui_tx
        .send_async(activity_event(
            request,
            agent_name,
            provider_id,
            model,
            status,
            detail,
        ))
        .await
        .map_err(|error| delivery_error(error, request, status))
}

fn activity_event(
    request: &StageRunRequest,
    agent_name: &str,
    provider_id: &str,
    model: &str,
    status: AgentActivityStatus,
    detail: &str,
) -> TuiEvent {
    TuiEvent::AgentActivity(AgentActivityUpdate {
        id: format!("workflow:{}:{}", request.run_id, request.stage_id),
        name: agent_name.to_string(),
        role: AgentActivityRole::Subagent,
        status,
        current_tool: None,
        detail: Some(activity_detail(request, detail)),
        run_id: Some(request.run_id.clone()),
        parent_id: None,
        artifact_id: None,
        provider: Some(provider_id.to_string()),
        model: Some(model.to_string()),
        cost_usd: None,
    })
}

fn delivery_error(
    error: tokio::sync::mpsc::error::SendError<TuiEvent>,
    request: &StageRunRequest,
    status: AgentActivityStatus,
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
