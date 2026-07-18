use super::tool_types::PreflightResult;
use super::*;

impl Agent {
    pub(super) async fn preflight_tools(
        &mut self,
        pending_tools: &[PendingToolCall],
        effective_mode: AgentMode,
    ) -> Vec<PreflightResult> {
        let mut allowed = Vec::new();
        for tool in pending_tools {
            if let Some(preflight) = self.preflight_single_tool(tool, effective_mode).await {
                allowed.push(preflight);
            }
        }
        if let (Some(callback), Some(action_id), Some(first)) = (
            &self.first_tool_action_callback,
            self.guardrail_action_id.as_deref(),
            allowed.first(),
        ) {
            callback(action_id, &first.tool_name, &first.tool_id, &first.input);
        }
        allowed
    }
}
