use super::*;
use crate::subagent::SubagentStatus;

pub(super) async fn read_result(ctx: &RouterContext, req: &SendMessageRequest) -> ToolResult {
    let manager = ctx.manager.lock().await;
    let Some(id) = manager.result_id(&req.to) else {
        return ToolResult::error(format!("No agent '{}' is known to this session", req.to));
    };
    let Some(info) = manager.get_status(id) else {
        return ToolResult::error(format!("No saved state for agent '{id}'"));
    };
    let (status, error) = match &info.status {
        SubagentStatus::Running => ("running", None),
        SubagentStatus::Completed => ("completed", None),
        SubagentStatus::TimedOut => ("timed_out", Some("agent timed out".to_string())),
        SubagentStatus::Failed(reason) => ("failed", Some(reason.clone())),
    };
    ToolResult::success(
        serde_json::json!({
            "agent_id": id, "status": status, "result": info.result, "error": error,
        })
        .to_string(),
    )
}
