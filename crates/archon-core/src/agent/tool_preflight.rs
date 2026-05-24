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
        allowed
    }
}
